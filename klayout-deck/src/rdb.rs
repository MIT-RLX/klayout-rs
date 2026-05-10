//! Reports Database (RDB) writer — KLayout-RDB XML format.
//!
//! Output is the same XML schema KLayout uses for `.lyrdb` files:
//! `<report-database>` containing `<categories>`, `<cells>`, and
//! `<items>`. KLayout's GUI opens these directly with markers and
//! categorized navigation. Reference: KLayout's `rdbFile.cc`.
//!
//! Each rule from a `DrcReport` becomes one category; each violation
//! polygon becomes one item with a `polygon: ...` value (using
//! KLayout's polygon-string format `(x0,y0;x1,y1;...)`, hull only —
//! holes via the `/` separator are not yet emitted).

use crate::DrcReport;
use klayout_core::Polygon;
use std::path::Path;

impl DrcReport {
    /// Serialize this report to a KLayout-RDB XML string.
    /// `top_cell` is recorded in the `<top-cell>` field — used by the
    /// KLayout GUI to scroll markers to the right view.
    pub fn to_rdb_xml(&self, top_cell: &str) -> String {
        let mut buf = String::new();
        buf.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
        buf.push_str("<report-database>\n");
        buf.push_str(" <description></description>\n");
        buf.push_str(" <original-file></original-file>\n");
        buf.push_str(" <generator>klayout-deck</generator>\n");
        buf.push_str(&format!(" <top-cell>{}</top-cell>\n", xml_escape(top_cell)));
        buf.push_str(" <tags></tags>\n");

        // Categories: one per rule.
        buf.push_str(" <categories>\n");
        for r in &self.results {
            buf.push_str("  <category>\n");
            buf.push_str(&format!(
                "   <name>{}</name>\n",
                xml_escape(&r.name)
            ));
            buf.push_str("   <description></description>\n");
            buf.push_str("  </category>\n");
        }
        buf.push_str(" </categories>\n");

        // Cells: just the top.
        buf.push_str(" <cells>\n");
        buf.push_str("  <cell>\n");
        buf.push_str(&format!(
            "   <name>{}</name>\n",
            xml_escape(top_cell)
        ));
        buf.push_str("   <variant>0</variant>\n");
        buf.push_str("  </cell>\n");
        buf.push_str(" </cells>\n");

        // Items: one per violation polygon.
        buf.push_str(" <items>\n");
        for r in &self.results {
            for poly in r.violations.polygons() {
                buf.push_str("  <item>\n");
                buf.push_str("   <tags></tags>\n");
                buf.push_str(&format!(
                    "   <category>{}</category>\n",
                    xml_escape(&r.name)
                ));
                buf.push_str(&format!(
                    "   <cell>{}</cell>\n",
                    xml_escape(top_cell)
                ));
                buf.push_str("   <visited>false</visited>\n");
                buf.push_str("   <multiplicity>1</multiplicity>\n");
                buf.push_str("   <values>\n");
                buf.push_str(&format!(
                    "    <value>polygon: {}</value>\n",
                    xml_escape(&polygon_to_klayout_string(poly))
                ));
                buf.push_str("   </values>\n");
                buf.push_str("  </item>\n");
            }
        }
        buf.push_str(" </items>\n");
        buf.push_str("</report-database>\n");
        buf
    }

    /// Write the report as a KLayout `.lyrdb` XML file.
    pub fn save_rdb(&self, top_cell: &str, path: impl AsRef<Path>) -> std::io::Result<()> {
        std::fs::write(path, self.to_rdb_xml(top_cell))
    }
}

/// KLayout polygon-string format: `(x0,y0;x1,y1;...;xN,yN)`. Holes
/// follow the hull separated by `/`. v1 emits hull-only polygons.
fn polygon_to_klayout_string(p: &Polygon) -> String {
    let mut s = String::with_capacity(p.hull.len() * 12);
    s.push('(');
    for (i, pt) in p.hull.iter().enumerate() {
        if i > 0 {
            s.push(';');
        }
        s.push_str(&format!("{},{}", pt.x, pt.y));
    }
    for hole in &p.holes {
        s.push('/');
        for (i, pt) in hole.iter().enumerate() {
            if i > 0 {
                s.push(';');
            }
            s.push_str(&format!("{},{}", pt.x, pt.y));
        }
    }
    s.push(')');
    s
}

fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}
