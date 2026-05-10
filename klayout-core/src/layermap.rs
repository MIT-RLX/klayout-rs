//! Layer mapping file (.layermap / .map / .lyp shorthand).
//!
//! IP integration always goes through a layer-name translation step:
//! foundry A's "M1" might be `(layer=10, datatype=0)` while foundry B
//! ships the same metal as `(layer=68, datatype=20)`. A `LayerMap`
//! captures the mapping so layouts can be re-keyed without manual
//! editing.
//!
//! The format we read/write is the de-facto industry plaintext form:
//!
//! ```text
//! # comment
//! METAL1   drawing  10  0
//! METAL1   pin      10  1
//! METAL1   label    10  2
//! VIA12    drawing  68  0
//! ```
//!
//! Each entry is `<name> <purpose> <layer> <datatype>`. `purpose` is a
//! free-form string (`drawing`, `pin`, `label`, `text`, `boundary`,
//! …); convention varies by foundry. Unknown purposes are preserved.
//!
//! Calibre/SVRF style maps and KLayout `.lyp` XML files are not v1
//! targets — those carry display metadata (color, fill) on top of the
//! mapping and would warrant their own crates.

use crate::layer::LayerInfo;
use smol_str::SmolStr;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerMapEntry {
    pub name: SmolStr,
    pub purpose: SmolStr,
    pub layer: u16,
    pub datatype: u16,
}

#[derive(Default, Clone, Debug)]
pub struct LayerMapping {
    pub entries: Vec<LayerMapEntry>,
    by_key: HashMap<(SmolStr, SmolStr), usize>,
    by_gds: HashMap<(u16, u16), usize>,
}

impl LayerMapping {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, entry: LayerMapEntry) {
        let idx = self.entries.len();
        self.by_key.insert((entry.name.clone(), entry.purpose.clone()), idx);
        self.by_gds.insert((entry.layer, entry.datatype), idx);
        self.entries.push(entry);
    }

    pub fn lookup_name(&self, name: &str, purpose: &str) -> Option<&LayerMapEntry> {
        let key = (SmolStr::from(name), SmolStr::from(purpose));
        self.by_key.get(&key).map(|&i| &self.entries[i])
    }

    pub fn lookup_gds(&self, layer: u16, datatype: u16) -> Option<&LayerMapEntry> {
        self.by_gds.get(&(layer, datatype)).map(|&i| &self.entries[i])
    }

    /// Convert an entry to a [`LayerInfo`] using `name` as the layer
    /// name. The `purpose` field is dropped — `LayerInfo` is keyed by
    /// (layer, datatype) only.
    pub fn to_layer_info(entry: &LayerMapEntry) -> LayerInfo {
        LayerInfo::named(entry.name.clone(), entry.layer, entry.datatype)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LayerMapError {
    #[error("layermap parse error on line {line}: {msg}")]
    Parse { line: usize, msg: String },
}

pub fn parse_layermap(text: &str) -> Result<LayerMapping, LayerMapError> {
    let mut map = LayerMapping::new();
    for (line_no, line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            continue;
        }
        // Split on whitespace; tolerate tabs.
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() < 4 {
            return Err(LayerMapError::Parse {
                line: line_no,
                msg: format!("expected 4 fields, got {}", parts.len()),
            });
        }
        let name = SmolStr::from(parts[0]);
        let purpose = SmolStr::from(parts[1]);
        let layer = parts[2].parse::<u16>().map_err(|_| LayerMapError::Parse {
            line: line_no,
            msg: format!("invalid layer number `{}`", parts[2]),
        })?;
        let datatype = parts[3].parse::<u16>().map_err(|_| LayerMapError::Parse {
            line: line_no,
            msg: format!("invalid datatype `{}`", parts[3]),
        })?;
        map.insert(LayerMapEntry {
            name,
            purpose,
            layer,
            datatype,
        });
    }
    Ok(map)
}

pub fn write_layermap(map: &LayerMapping) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "# name  purpose  layer  datatype");
    // Determine column widths so the output is aligned.
    let name_w = map
        .entries
        .iter()
        .map(|e| e.name.len())
        .max()
        .unwrap_or(0)
        .max(4);
    let purp_w = map
        .entries
        .iter()
        .map(|e| e.purpose.len())
        .max()
        .unwrap_or(0)
        .max(7);
    for e in &map.entries {
        let _ = writeln!(
            out,
            "{:<name_w$} {:<purp_w$} {:>3} {:>3}",
            e.name.as_str(),
            e.purpose.as_str(),
            e.layer,
            e.datatype,
            name_w = name_w,
            purp_w = purp_w,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
# Foundry XYZ map
METAL1   drawing  10  0
METAL1   pin      10  1
METAL1   label    10  2
VIA12    drawing  68  0
POLY     drawing   7  0
"#;

    #[test]
    fn parses_basic_map() {
        let map = parse_layermap(SAMPLE).unwrap();
        assert_eq!(map.entries.len(), 5);
        let m1 = map.lookup_name("METAL1", "drawing").unwrap();
        assert_eq!(m1.layer, 10);
        assert_eq!(m1.datatype, 0);
    }

    #[test]
    fn lookup_by_gds_pair() {
        let map = parse_layermap(SAMPLE).unwrap();
        let v = map.lookup_gds(68, 0).unwrap();
        assert_eq!(v.name.as_str(), "VIA12");
    }

    #[test]
    fn comments_and_blank_lines_skipped() {
        let txt = "# header\n\nA drawing 1 0\n# end\n";
        let map = parse_layermap(txt).unwrap();
        assert_eq!(map.entries.len(), 1);
    }

    #[test]
    fn malformed_rejected() {
        let bad = "BAD line\n";
        assert!(parse_layermap(bad).is_err());
        let bad2 = "X drawing notanumber 0\n";
        assert!(parse_layermap(bad2).is_err());
    }

    #[test]
    fn round_trip_via_string() {
        let map1 = parse_layermap(SAMPLE).unwrap();
        let text = write_layermap(&map1);
        let map2 = parse_layermap(&text).unwrap();
        assert_eq!(map2.entries.len(), map1.entries.len());
        for e1 in &map1.entries {
            let e2 = map2.lookup_name(&e1.name, &e1.purpose).unwrap();
            assert_eq!(e1.layer, e2.layer);
            assert_eq!(e1.datatype, e2.datatype);
        }
    }

    #[test]
    fn entry_to_layer_info() {
        let entry = LayerMapEntry {
            name: "METAL1".into(),
            purpose: "drawing".into(),
            layer: 10,
            datatype: 0,
        };
        let info = LayerMapping::to_layer_info(&entry);
        assert_eq!(info.name.as_str(), "METAL1");
        assert_eq!(info.layer, 10);
    }
}
