//! KLayout `.lyt` technology file.
//!
//! Companion to `.lyp`: while `.lyp` configures display, `.lyt`
//! configures the tech (DBU, default layer table, hierarchy levels,
//! library name). KLayout uses it to set up a fresh layout view.
//!
//! v1 captures the structural fields the GUI / batch flows actually
//! read: `name`, `description`, `dbu`, the embedded layer table
//! (which is just a `Lyp` document under a different root tag),
//! `default-grids`, `connectivity` placeholders.

use crate::lyp::{parse_lyp, write_lyp, Lyp, LypError};
use smol_str::SmolStr;
use std::path::Path;

#[derive(Default, Clone, Debug)]
pub struct Lyt {
    pub name: SmolStr,
    pub description: SmolStr,
    pub dbu: f64,
    pub default_grids: SmolStr,
    pub layer_properties: Lyp,
}

pub fn read_lyt_path(path: impl AsRef<Path>) -> Result<Lyt, LypError> {
    let s = std::fs::read_to_string(path)?;
    parse_lyt(&s)
}

pub fn parse_lyt(text: &str) -> Result<Lyt, LypError> {
    let layer_properties =
        if let Some((_, body_start)) = xml::find_open(text, "layer-properties", 0) {
            if let Some((body_end, _)) = xml::find_close(text, "layer-properties", body_start) {
                let body = &text[body_start..body_end];
                let pseudo = format!("<layer-properties>{body}</layer-properties>");
                parse_lyp(&pseudo)?
            } else {
                Lyp::default()
            }
        } else {
            Lyp::default()
        };
    Ok(Lyt {
        name: SmolStr::from(xml::tag_text(text, "name").unwrap_or_default()),
        description: SmolStr::from(xml::tag_text(text, "description").unwrap_or_default()),
        dbu: xml::tag_text(text, "dbu")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.001),
        default_grids: SmolStr::from(xml::tag_text(text, "default-grids").unwrap_or_default()),
        layer_properties,
    })
}

pub fn write_lyt(t: &Lyt) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "<?xml version=\"1.0\" encoding=\"utf-8\"?>");
    let _ = writeln!(s, "<technology>");
    let _ = writeln!(s, " <name>{}</name>", t.name);
    let _ = writeln!(s, " <description>{}</description>", t.description);
    let _ = writeln!(s, " <dbu>{}</dbu>", t.dbu);
    let _ = writeln!(s, " <default-grids>{}</default-grids>", t.default_grids);
    // Inline the layer table — strip the <?xml?> prologue first.
    let lyp_text = write_lyp(&t.layer_properties);
    let lyp_body: String = lyp_text.split_inclusive('\n').skip(1).collect();
    s.push_str(&lyp_body);
    let _ = writeln!(s, "</technology>");
    s
}

/// Tiny line-based XML helpers shared by .lyp / .lyt. Not a general
/// XML parser — it tolerates only the tag soup KLayout emits.
pub mod xml {
    pub fn find_open(text: &str, tag: &str, from: usize) -> Option<(usize, usize)> {
        let needle = format!("<{tag}>");
        text[from..]
            .find(&needle)
            .map(|i| (from + i, from + i + needle.len()))
    }

    pub fn find_close(text: &str, tag: &str, from: usize) -> Option<(usize, usize)> {
        let needle = format!("</{tag}>");
        text[from..]
            .find(&needle)
            .map(|i| (from + i, from + i + needle.len()))
    }

    pub fn tag_text(text: &str, tag: &str) -> Option<String> {
        let (open_start, open_end) = find_open(text, tag, 0)?;
        let _ = open_start;
        let (close_start, _close_end) = find_close(text, tag, open_end)?;
        Some(text[open_end..close_start].trim().to_string())
    }

    pub fn tag_bool(text: &str, tag: &str) -> Option<bool> {
        tag_text(text, tag).map(|s| s.eq_ignore_ascii_case("true"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<technology>
 <name>foundry-tech</name>
 <description>Test technology</description>
 <dbu>0.001</dbu>
 <default-grids>0.005,0.01</default-grids>
 <layer-properties>
  <properties>
   <name>METAL1</name>
   <source>10/0@1</source>
   <frame-color>#ff0000</frame-color>
   <fill-color>#ff8080</fill-color>
   <valid>true</valid>
   <visible>true</visible>
  </properties>
 </layer-properties>
</technology>"#;

    #[test]
    fn parses_basic_tech() {
        let t = parse_lyt(SAMPLE).unwrap();
        assert_eq!(t.name.as_str(), "foundry-tech");
        assert_eq!(t.description.as_str(), "Test technology");
        assert!((t.dbu - 0.001).abs() < 1e-9);
        assert_eq!(t.default_grids.as_str(), "0.005,0.01");
        assert_eq!(t.layer_properties.layers.len(), 1);
        assert_eq!(t.layer_properties.layers[0].name.as_str(), "METAL1");
    }

    #[test]
    fn round_trip_via_string() {
        let t1 = parse_lyt(SAMPLE).unwrap();
        let text = write_lyt(&t1);
        let t2 = parse_lyt(&text).unwrap();
        assert_eq!(t1.name, t2.name);
        assert_eq!(
            t1.layer_properties.layers.len(),
            t2.layer_properties.layers.len()
        );
    }
}
