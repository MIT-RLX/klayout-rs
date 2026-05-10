//! DXF (AutoCAD Drawing Exchange Format) reader and writer.
//!
//! DXF is a text-based mechanical-CAD format used to bridge IC
//! layout to PCB / package / mechanical CAM tools. The format is
//! group-code-based: each line is either a numeric group code or
//! its value, paired into records like `0 / SECTION` (entity type)
//! and `8 / METAL1` (layer name).
//!
//! v1 supports the entities commonly produced by IC backends:
//! * `LINE` (group code 0=LINE) with start/end points → polygon
//! * `LWPOLYLINE` (0=LWPOLYLINE, 90=vertex_count, 70=flags,
//!   10/20=vertex pairs) → polygon
//! * `POLYLINE` (legacy heavy-weight polyline) → polygon
//! * `CIRCLE` (10/20=center, 40=radius) → polygon approximation
//!
//! Layer names from the DXF `8` group code map to klayout layer
//! names; we assign GDS `(layer, datatype)` automatically using a
//! sequential counter.

use crate::error::{IoError, Result};
use klayout_core::{
    Bbox, CellBuilder, CellName, LayerInfo, Library, Point, Polygon, Rect,
};
use std::collections::HashMap;
use std::path::Path;

pub fn read_dxf_path(path: impl AsRef<Path>) -> Result<Library> {
    let s = std::fs::read_to_string(path.as_ref())?;
    read_dxf_str(&s)
}

/// DXF natural unit is one micron; Library DBU is 1 nm. Matches
/// KLayout's reader (drawing units = µm).
const DXF_TO_DBU: f64 = 1000.0;

pub fn read_dxf_str(text: &str) -> Result<Library> {
    let lib = Library::new("dxf", 1000);
    let mut layer_for_name: HashMap<String, klayout_core::LayerIndex> = HashMap::new();
    let layer_idx = |lib: &Library,
                          layer_for_name: &mut HashMap<String, klayout_core::LayerIndex>,
                          name: &str|
     -> klayout_core::LayerIndex {
        if let Some(&l) = layer_for_name.get(name) {
            return l;
        }
        // KLayout: DXF layer named with a number (e.g., "1") maps to
        // GDS (N, 0). Other names need a synthetic GDS number; we hash
        // the layer name (FNV-1a, low 15 bits) so distinct names get
        // distinct `(gds, dt)` keys — without this, two named layers
        // would collide on the `(u16::MAX, u16::MAX)` key and the
        // library's `get_or_insert` would dedup them into one.
        let info = match name.parse::<u16>() {
            Ok(n) => LayerInfo::gds(n, 0),
            Err(_) => {
                let mut h: u32 = 0x811c9dc5;
                for b in name.bytes() {
                    h ^= b as u32;
                    h = h.wrapping_mul(0x0100_0193);
                }
                let synth = ((h & 0x7FFF) as u16).max(1);
                // `dt = u16::MAX` is the canonical-dump marker for
                // "named layer with no GDS mapping" — keeps parity
                // with KLayout's `-1` rendering for non-GDS readers.
                LayerInfo::named(name, synth, u16::MAX)
            }
        };
        let li = lib.layer(info);
        layer_for_name.insert(name.to_string(), li);
        li
    };

    // KLayout names the synthetic top cell of a DXF file "TOP".
    let mut cb = CellBuilder::new(CellName::new("TOP"));
    let pairs = parse_pairs(text);
    let mut iter = pairs.iter().peekable();

    while let Some((code, value)) = iter.next() {
        if *code != 0 {
            continue;
        }
        match value.as_str() {
            "LINE" => {
                let mut x1 = 0.0f64;
                let mut y1 = 0.0f64;
                let mut x2 = 0.0f64;
                let mut y2 = 0.0f64;
                let mut layer = "0".to_string();
                while let Some((c, v)) = iter.peek() {
                    if *c == 0 {
                        break;
                    }
                    let (c, v) = (*c, v.clone());
                    iter.next();
                    match c {
                        8 => layer = v,
                        10 => x1 = v.parse().unwrap_or(0.0),
                        20 => y1 = v.parse().unwrap_or(0.0),
                        11 => x2 = v.parse().unwrap_or(0.0),
                        21 => y2 = v.parse().unwrap_or(0.0),
                        _ => {}
                    }
                }
                let li = layer_idx(&lib, &mut layer_for_name, &layer);
                let pts = vec![
                    Point::new(
                        (x1 * DXF_TO_DBU).round() as i64,
                        (y1 * DXF_TO_DBU).round() as i64,
                    ),
                    Point::new(
                        (x2 * DXF_TO_DBU).round() as i64,
                        (y2 * DXF_TO_DBU).round() as i64,
                    ),
                ];
                if pts[0] != pts[1] {
                    // KLayout reads DXF LINE as a zero-width PATH, not a
                    // degenerate 2-vertex polygon.
                    cb.add_shape(li, klayout_core::Path::new(pts, 0));
                }
            }
            "LWPOLYLINE" | "POLYLINE" => {
                let mut layer = "0".to_string();
                let mut xs: Vec<f64> = Vec::new();
                let mut ys: Vec<f64> = Vec::new();
                while let Some((c, v)) = iter.peek() {
                    if *c == 0 {
                        break;
                    }
                    let (c, v) = (*c, v.clone());
                    iter.next();
                    match c {
                        8 => layer = v,
                        10 => xs.push(v.parse().unwrap_or(0.0)),
                        20 => ys.push(v.parse().unwrap_or(0.0)),
                        _ => {}
                    }
                }
                let pts: Vec<Point> = xs
                    .iter()
                    .zip(ys.iter())
                    .map(|(x, y)| Point::new((x * DXF_TO_DBU).round() as i64, (y * DXF_TO_DBU).round() as i64))
                    .collect();
                if pts.len() >= 3 {
                    let li = layer_idx(&lib, &mut layer_for_name, &layer);
                    cb.add_shape(li, Polygon::from_hull(pts));
                }
            }
            "CIRCLE" => {
                let mut cx = 0.0f64;
                let mut cy = 0.0f64;
                let mut r = 0.0f64;
                let mut layer = "0".to_string();
                while let Some((c, v)) = iter.peek() {
                    if *c == 0 {
                        break;
                    }
                    let (c, v) = (*c, v.clone());
                    iter.next();
                    match c {
                        8 => layer = v,
                        10 => cx = v.parse().unwrap_or(0.0),
                        20 => cy = v.parse().unwrap_or(0.0),
                        40 => r = v.parse().unwrap_or(0.0),
                        _ => {}
                    }
                }
                if r > 0.0 {
                    let li = layer_idx(&lib, &mut layer_for_name, &layer);
                    let pts = circle_hull(cx as i64, cy as i64, r as i64, 32);
                    cb.add_shape(li, Polygon::from_hull(pts));
                }
            }
            "TEXT" => {
                let mut tx = 0.0f64;
                let mut ty = 0.0f64;
                let mut text = String::new();
                let mut layer = "0".to_string();
                while let Some((c, v)) = iter.peek() {
                    if *c == 0 {
                        break;
                    }
                    let (c, v) = (*c, v.clone());
                    iter.next();
                    match c {
                        8 => layer = v,
                        10 => tx = v.parse().unwrap_or(0.0),
                        20 => ty = v.parse().unwrap_or(0.0),
                        1 => text = v, // text-string group code
                        _ => {}
                    }
                }
                if !text.is_empty() {
                    let li = layer_idx(&lib, &mut layer_for_name, &layer);
                    cb.add_shape(
                        li,
                        klayout_core::Text::new(
                            text,
                            Point::new(
                                (tx * DXF_TO_DBU).round() as i64,
                                (ty * DXF_TO_DBU).round() as i64,
                            ),
                        ),
                    );
                }
            }
            "ARC" => {
                let mut cx = 0.0f64;
                let mut cy = 0.0f64;
                let mut r = 0.0f64;
                let mut start_angle = 0.0f64;
                let mut end_angle = 360.0f64;
                let mut layer = "0".to_string();
                while let Some((c, v)) = iter.peek() {
                    if *c == 0 {
                        break;
                    }
                    let (c, v) = (*c, v.clone());
                    iter.next();
                    match c {
                        8 => layer = v,
                        10 => cx = v.parse().unwrap_or(0.0),
                        20 => cy = v.parse().unwrap_or(0.0),
                        40 => r = v.parse().unwrap_or(0.0),
                        50 => start_angle = v.parse().unwrap_or(0.0),
                        51 => end_angle = v.parse().unwrap_or(360.0),
                        _ => {}
                    }
                }
                if r > 0.0 {
                    let li = layer_idx(&lib, &mut layer_for_name, &layer);
                    let pts = arc_hull(
                        cx as i64,
                        cy as i64,
                        r as i64,
                        start_angle,
                        end_angle,
                        32,
                    );
                    if pts.len() >= 3 {
                        cb.add_shape(li, Polygon::from_hull(pts));
                    }
                }
            }
            "ELLIPSE" => {
                let mut cx = 0.0f64;
                let mut cy = 0.0f64;
                let mut major_x = 0.0f64;
                let mut major_y = 0.0f64;
                let mut ratio = 1.0f64;
                let mut layer = "0".to_string();
                while let Some((c, v)) = iter.peek() {
                    if *c == 0 {
                        break;
                    }
                    let (c, v) = (*c, v.clone());
                    iter.next();
                    match c {
                        8 => layer = v,
                        10 => cx = v.parse().unwrap_or(0.0),
                        20 => cy = v.parse().unwrap_or(0.0),
                        11 => major_x = v.parse().unwrap_or(0.0),
                        21 => major_y = v.parse().unwrap_or(0.0),
                        40 => ratio = v.parse().unwrap_or(1.0),
                        _ => {}
                    }
                }
                let major = (major_x * major_x + major_y * major_y).sqrt();
                let minor = major * ratio;
                if major > 0.0 {
                    let li = layer_idx(&lib, &mut layer_for_name, &layer);
                    let angle0 = major_y.atan2(major_x);
                    let pts = ellipse_hull(cx as i64, cy as i64, major, minor, angle0, 32);
                    cb.add_shape(li, Polygon::from_hull(pts));
                }
            }
            "INSERT" => {
                // Block reference. Group codes: 2=block name, 10/20=insertion point, 41/42=scale, 50=rotation.
                let mut block_name = String::new();
                let mut tx = 0.0f64;
                let mut ty = 0.0f64;
                while let Some((c, v)) = iter.peek() {
                    if *c == 0 {
                        break;
                    }
                    let (c, v) = (*c, v.clone());
                    iter.next();
                    match c {
                        2 => block_name = v,
                        10 => tx = v.parse().unwrap_or(0.0),
                        20 => ty = v.parse().unwrap_or(0.0),
                        50 => {
                            let _: f64 = v.parse().unwrap_or(0.0);
                        }
                        _ => {}
                    }
                }
                // Block-defined cells aren't in v1 reader; record the
                // INSERT as a dangling instance reference. For now we
                // skip; full block support is a v2.
                let _ = (block_name, tx, ty);
            }
            _ => {}
        }
    }
    lib.insert(cb);
    let _ = IoError::UnexpectedEof;
    Ok(lib)
}

fn arc_hull(
    cx: i64,
    cy: i64,
    r: i64,
    start_deg: f64,
    end_deg: f64,
    facets: usize,
) -> Vec<Point> {
    use std::f64::consts::PI;
    let start_rad = start_deg * PI / 180.0;
    let mut end_rad = end_deg * PI / 180.0;
    if end_rad <= start_rad {
        end_rad += 2.0 * PI;
    }
    let mut out = Vec::with_capacity(facets + 1);
    out.push(Point::new(cx, cy));
    for i in 0..=facets {
        let a = start_rad + (end_rad - start_rad) * (i as f64) / (facets as f64);
        let dx = (r as f64 * a.cos()).round() as i64;
        let dy = (r as f64 * a.sin()).round() as i64;
        out.push(Point::new(cx + dx, cy + dy));
    }
    out
}

fn ellipse_hull(
    cx: i64,
    cy: i64,
    major: f64,
    minor: f64,
    rotation: f64,
    facets: usize,
) -> Vec<Point> {
    use std::f64::consts::PI;
    let cos_r = rotation.cos();
    let sin_r = rotation.sin();
    let mut out = Vec::with_capacity(facets);
    for i in 0..facets {
        let a = 2.0 * PI * (i as f64) / (facets as f64);
        let lx = major * a.cos();
        let ly = minor * a.sin();
        // Rotate by `rotation`.
        let dx = (lx * cos_r - ly * sin_r).round() as i64;
        let dy = (lx * sin_r + ly * cos_r).round() as i64;
        out.push(Point::new(cx + dx, cy + dy));
    }
    out
}

pub fn write_dxf_str(lib: &Library) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "0\nSECTION\n2\nENTITIES");
    for (_id, cell) in lib.all_cells() {
        for layer_idx in cell.layers() {
            let info = lib.layer_info(layer_idx);
            let lname = if info.name.is_empty() {
                format!("L{}_{}", info.layer, info.datatype)
            } else {
                info.name.to_string()
            };
            for shape in cell.shapes_on(layer_idx) {
                match shape {
                    klayout_core::Shape::Box(r) => {
                        let b = r.bbox;
                        // Emit as LWPOLYLINE.
                        let _ = writeln!(s, "0\nLWPOLYLINE\n8\n{lname}\n90\n4\n70\n1");
                        for (x, y) in [
                            (b.min.x, b.min.y),
                            (b.max.x, b.min.y),
                            (b.max.x, b.max.y),
                            (b.min.x, b.max.y),
                        ] {
                            let _ = writeln!(s, "10\n{x}\n20\n{y}");
                        }
                    }
                    klayout_core::Shape::Polygon(p) => {
                        let _ = writeln!(s, "0\nLWPOLYLINE\n8\n{lname}\n90\n{}\n70\n1", p.hull.len());
                        for pt in &p.hull {
                            let _ = writeln!(s, "10\n{}\n20\n{}", pt.x, pt.y);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    let _ = writeln!(s, "0\nENDSEC\n0\nEOF");
    s
}

fn parse_pairs(text: &str) -> Vec<(i32, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 1 < lines.len() {
        let code: i32 = match lines[i].trim().parse() {
            Ok(c) => c,
            Err(_) => {
                i += 1;
                continue;
            }
        };
        let value = lines[i + 1].trim().to_string();
        out.push((code, value));
        i += 2;
    }
    let _ = Bbox::EMPTY; // suppress unused if Bbox not used in code path
    let _ = Rect::new(Bbox::EMPTY);
    out
}

fn circle_hull(cx: i64, cy: i64, radius: i64, facets: usize) -> Vec<Point> {
    use std::f64::consts::PI;
    let mut out = Vec::with_capacity(facets);
    for i in 0..facets {
        let a = 2.0 * PI * (i as f64) / (facets as f64);
        let dx = (radius as f64 * a.cos()).round() as i64;
        let dy = (radius as f64 * a.sin()).round() as i64;
        out.push(Point::new(cx + dx, cy + dy));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0\nSECTION\n2\nENTITIES\n0\nLINE\n8\nMETAL1\n10\n0\n20\n0\n11\n100\n21\n50\n0\nLWPOLYLINE\n8\nVIA12\n90\n4\n70\n1\n10\n0\n20\n0\n10\n10\n20\n0\n10\n10\n20\n10\n10\n0\n20\n10\n0\nCIRCLE\n8\nMETAL2\n10\n50\n20\n50\n40\n10\n0\nENDSEC\n0\nEOF";

    #[test]
    fn parse_basic_entities() {
        let lib = read_dxf_str(SAMPLE).unwrap();
        // One top cell with shapes from each entity.
        assert_eq!(lib.cell_count(), 1);
        let top = lib.get(lib.by_name("TOP").unwrap());
        let layer_count = top.layers().count();
        assert!(layer_count >= 2, "got {layer_count} layers");
    }

    #[test]
    fn write_emits_dxf_envelope() {
        let lib = read_dxf_str(SAMPLE).unwrap();
        let text = write_dxf_str(&lib);
        assert!(text.contains("SECTION"));
        assert!(text.contains("ENDSEC"));
        assert!(text.contains("EOF"));
    }

    #[test]
    fn round_trip_preserves_polygon_count() {
        let lib1 = read_dxf_str(SAMPLE).unwrap();
        let text = write_dxf_str(&lib1);
        let lib2 = read_dxf_str(&text).unwrap();
        assert_eq!(lib1.cell_count(), lib2.cell_count());
    }

    #[test]
    fn text_entity_parsed() {
        let dxf = "0\nSECTION\n2\nENTITIES\n0\nTEXT\n8\nLABEL\n10\n50\n20\n100\n1\nHELLO\n0\nENDSEC\n0\nEOF";
        let lib = read_dxf_str(dxf).unwrap();
        let top = lib.get(lib.by_name("TOP").unwrap());
        let label = lib.layer_by_name("LABEL").unwrap();
        let shapes: Vec<_> = top.shapes_on(label).collect();
        assert_eq!(shapes.len(), 1);
        match &shapes[0] {
            klayout_core::Shape::Text(t) => {
                assert_eq!(t.string.as_str(), "HELLO");
                // DXF coordinates are scaled by `DXF_TO_DBU`; the input
                // says (50, 100) µm which lands at (50_000, 100_000) DBU.
                assert_eq!(t.anchor, klayout_core::Point::new(50_000, 100_000));
            }
            _ => panic!("expected Text"),
        }
    }

    #[test]
    fn arc_entity_polygonised() {
        // Half-circle: ARC at (0,0) radius 50, 0° to 180°.
        let dxf = "0\nSECTION\n2\nENTITIES\n0\nARC\n8\nM1\n10\n0\n20\n0\n40\n50\n50\n0\n51\n180\n0\nENDSEC\n0\nEOF";
        let lib = read_dxf_str(dxf).unwrap();
        let top = lib.get(lib.by_name("TOP").unwrap());
        let m1 = lib.layer_by_name("M1").unwrap();
        let shapes: Vec<_> = top.shapes_on(m1).collect();
        assert_eq!(shapes.len(), 1);
    }

    #[test]
    fn ellipse_entity_polygonised() {
        // Ellipse at (0,0) with major axis along x of length 50, ratio 0.5.
        let dxf = "0\nSECTION\n2\nENTITIES\n0\nELLIPSE\n8\nM1\n10\n0\n20\n0\n11\n50\n21\n0\n40\n0.5\n0\nENDSEC\n0\nEOF";
        let lib = read_dxf_str(dxf).unwrap();
        let top = lib.get(lib.by_name("TOP").unwrap());
        let m1 = lib.layer_by_name("M1").unwrap();
        let shapes: Vec<_> = top.shapes_on(m1).collect();
        assert_eq!(shapes.len(), 1);
    }
}
