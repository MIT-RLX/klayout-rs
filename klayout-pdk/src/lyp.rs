//! KLayout `.lyp` layer-properties file.
//!
//! `.lyp` is KLayout's per-layer display config: color, fill pattern,
//! frame width, visibility. It's an XML format with one
//! `<properties>` block per layer/datatype pair. KLayout's GUI reads
//! it to render layouts; tools that emit GDS / OASIS often ship a
//! companion `.lyp` so the layout opens with sensible colours.
//!
//! v1 supports the common subset: name, source `(layer/datatype@cell)`,
//! `frame-color`, `fill-color`, `dither-pattern`, `visible`, `valid`,
//! `transparent`, `width`, `marked`, `xfill`, `animation`. Nested
//! `<group-members>` (logical layer groups) are flattened to the top
//! level on read; on write we emit a flat list.

use crate::lyt::xml; // re-use the tiny XML helpers
use smol_str::SmolStr;
use std::path::Path;

#[derive(Default, Clone, Debug)]
pub struct LayerProperties {
    pub name: SmolStr,
    pub source: SmolStr,
    pub frame_color: SmolStr,
    pub fill_color: SmolStr,
    pub frame_brightness: i32,
    pub fill_brightness: i32,
    pub dither_pattern: i32,
    pub line_style: i32,
    pub valid: bool,
    pub visible: bool,
    pub transparent: bool,
    pub width: i32,
    pub marked: bool,
    pub xfill: bool,
    pub animation: i32,
}

impl LayerProperties {
    pub fn new(name: impl Into<SmolStr>, layer: u16, datatype: u16) -> Self {
        Self {
            name: name.into(),
            source: SmolStr::from(format!("{layer}/{datatype}@1")),
            frame_color: SmolStr::from("#000000"),
            fill_color: SmolStr::from("#cccccc"),
            valid: true,
            visible: true,
            ..Default::default()
        }
    }
}

#[derive(Default, Clone, Debug)]
pub struct Lyp {
    pub layers: Vec<LayerProperties>,
}

#[derive(Debug)]
pub enum LypError {
    Io(std::io::Error),
    Parse(String),
}

impl From<std::io::Error> for LypError {
    fn from(e: std::io::Error) -> Self {
        LypError::Io(e)
    }
}

impl std::fmt::Display for LypError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LypError::Io(e) => write!(f, "io: {e}"),
            LypError::Parse(s) => write!(f, "parse: {s}"),
        }
    }
}
impl std::error::Error for LypError {}

pub fn read_lyp_path(path: impl AsRef<Path>) -> Result<Lyp, LypError> {
    let s = std::fs::read_to_string(path)?;
    parse_lyp(&s)
}

pub fn parse_lyp(text: &str) -> Result<Lyp, LypError> {
    let mut layers: Vec<LayerProperties> = Vec::new();
    let mut pos = 0;
    while let Some(start) = xml::find_open(text, "properties", pos) {
        let end = xml::find_close(text, "properties", start.1)
            .ok_or_else(|| LypError::Parse("unterminated <properties>".to_string()))?;
        let body = &text[start.1..end.0];
        layers.push(parse_one(body));
        pos = end.1;
    }
    Ok(Lyp { layers })
}

fn parse_one(body: &str) -> LayerProperties {
    LayerProperties {
        name: SmolStr::from(xml::tag_text(body, "name").unwrap_or_default()),
        source: SmolStr::from(xml::tag_text(body, "source").unwrap_or_default()),
        frame_color: SmolStr::from(xml::tag_text(body, "frame-color").unwrap_or_default()),
        fill_color: SmolStr::from(xml::tag_text(body, "fill-color").unwrap_or_default()),
        frame_brightness: xml::tag_text(body, "frame-brightness")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        fill_brightness: xml::tag_text(body, "fill-brightness")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        dither_pattern: xml::tag_text(body, "dither-pattern")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        line_style: xml::tag_text(body, "line-style")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        valid: xml::tag_bool(body, "valid").unwrap_or(true),
        visible: xml::tag_bool(body, "visible").unwrap_or(true),
        transparent: xml::tag_bool(body, "transparent").unwrap_or(false),
        width: xml::tag_text(body, "width")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1),
        marked: xml::tag_bool(body, "marked").unwrap_or(false),
        xfill: xml::tag_bool(body, "xfill").unwrap_or(false),
        animation: xml::tag_text(body, "animation")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
    }
}

pub fn write_lyp(lyp: &Lyp) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "<?xml version=\"1.0\" encoding=\"utf-8\"?>");
    let _ = writeln!(s, "<layer-properties>");
    for p in &lyp.layers {
        let _ = writeln!(s, " <properties>");
        emit_field(&mut s, "frame-color", &p.frame_color);
        emit_field(&mut s, "fill-color", &p.fill_color);
        emit_field(&mut s, "frame-brightness", &p.frame_brightness.to_string());
        emit_field(&mut s, "fill-brightness", &p.fill_brightness.to_string());
        emit_field(&mut s, "dither-pattern", &p.dither_pattern.to_string());
        emit_field(&mut s, "line-style", &p.line_style.to_string());
        emit_field(&mut s, "valid", bool_str(p.valid));
        emit_field(&mut s, "visible", bool_str(p.visible));
        emit_field(&mut s, "transparent", bool_str(p.transparent));
        emit_field(&mut s, "width", &p.width.to_string());
        emit_field(&mut s, "marked", bool_str(p.marked));
        emit_field(&mut s, "xfill", bool_str(p.xfill));
        emit_field(&mut s, "animation", &p.animation.to_string());
        emit_field(&mut s, "name", &p.name);
        emit_field(&mut s, "source", &p.source);
        let _ = writeln!(s, " </properties>");
    }
    let _ = writeln!(s, "</layer-properties>");
    s
}

fn emit_field(out: &mut String, tag: &str, value: &str) {
    use std::fmt::Write as _;
    let _ = writeln!(out, "  <{tag}>{value}</{tag}>");
}

fn bool_str(b: bool) -> &'static str {
    if b {
        "true"
    } else {
        "false"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<layer-properties>
 <properties>
  <frame-color>#ff0000</frame-color>
  <fill-color>#ff8080</fill-color>
  <frame-brightness>0</frame-brightness>
  <fill-brightness>0</fill-brightness>
  <dither-pattern>2</dither-pattern>
  <valid>true</valid>
  <visible>true</visible>
  <transparent>false</transparent>
  <width>1</width>
  <name>METAL1</name>
  <source>10/0@1</source>
 </properties>
 <properties>
  <frame-color>#00ff00</frame-color>
  <fill-color>#80ff80</fill-color>
  <name>VIA12</name>
  <source>11/0@1</source>
 </properties>
</layer-properties>"#;

    #[test]
    fn parses_two_layers() {
        let l = parse_lyp(SAMPLE).unwrap();
        assert_eq!(l.layers.len(), 2);
        assert_eq!(l.layers[0].name.as_str(), "METAL1");
        assert_eq!(l.layers[0].source.as_str(), "10/0@1");
        assert_eq!(l.layers[1].name.as_str(), "VIA12");
    }

    #[test]
    fn writes_well_formed_xml() {
        let l = parse_lyp(SAMPLE).unwrap();
        let text = write_lyp(&l);
        let l2 = parse_lyp(&text).unwrap();
        assert_eq!(l2.layers.len(), 2);
        assert_eq!(l2.layers[0].name.as_str(), "METAL1");
        assert_eq!(l2.layers[1].source.as_str(), "11/0@1");
    }

    #[test]
    fn defaults_apply_when_fields_missing() {
        let txt = r#"<layer-properties>
 <properties>
  <name>X</name>
  <source>1/0@1</source>
 </properties>
</layer-properties>"#;
        let l = parse_lyp(txt).unwrap();
        assert!(l.layers[0].valid);
        assert!(l.layers[0].visible);
        assert_eq!(l.layers[0].width, 1);
    }
}
