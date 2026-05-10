//! Magic (MAG) layout file reader.
//!
//! Magic is the open-source analog/custom layout editor still used
//! widely in academic-foundry and analog flows. Its native `.mag`
//! format is line-based ASCII:
//!
//! ```text
//! magic
//! tech sky130A
//! magscale 1 1
//! timestamp 1234567890
//! << layer >>
//! rect xll yll xur yur
//! rect ...
//! << end >>
//! use cellname
//! timestamp 1234567890
//! transform 1 0 0 0 1 0
//! box xll yll xur yur
//! << end >>
//! ```
//!
//! v1 reads the basic header + per-layer rect lists + cell-instance
//! `use` blocks. Writing MAG is asymmetric — most flows go GDS/OASIS
//! → Magic only when authoring; we don't ship a writer in v1.

use crate::error::{IoError, Result};
use klayout_core::{
    Bbox, CellBuilder, CellId, CellName, LayerInfo, Library, Point, Trans, Vec2,
};
use std::collections::HashMap;
use std::path::Path;

pub fn read_mag_path(path: impl AsRef<Path>) -> Result<Library> {
    let p = path.as_ref();
    let s = std::fs::read_to_string(p)?;
    let name = p
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("MAG_TOP")
        .to_string();
    read_mag_named(&s, &name)
}

/// Emit a `Library` as Magic .mag text.
pub fn write_mag_str(lib: &Library, tech: &str) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (_id, cell) in lib.all_cells() {
        let _ = writeln!(out, "magic");
        let _ = writeln!(out, "tech {tech}");
        let _ = writeln!(out, "magscale 1 1");
        let _ = writeln!(out, "timestamp 0");
        for layer_idx in cell.layers() {
            let info = lib.layer_info(layer_idx);
            let lname = if info.name.is_empty() {
                format!("L{}_{}", info.layer, info.datatype)
            } else {
                info.name.to_string()
            };
            let mut emitted_header = false;
            for shape in cell.shapes_on(layer_idx) {
                let bb = match shape {
                    klayout_core::Shape::Box(r) => Some(r.bbox),
                    klayout_core::Shape::Polygon(p) => Some(p.bbox()),
                    _ => None,
                };
                if let Some(b) = bb {
                    if !emitted_header {
                        let _ = writeln!(out, "<< {lname} >>");
                        emitted_header = true;
                    }
                    let _ = writeln!(
                        out,
                        "rect {} {} {} {}",
                        b.min.x, b.min.y, b.max.x, b.max.y
                    );
                }
            }
            if emitted_header {
                let _ = writeln!(out, "<< end >>");
            }
        }
        for inst in cell.instances() {
            let child_name = lib.get(inst.cell).name().to_string();
            let _ = writeln!(out, "use {child_name}");
            let _ = writeln!(out, "timestamp 0");
            let tx = inst.trans.disp.x;
            let ty = inst.trans.disp.y;
            let _ = writeln!(out, "transform 1 0 {tx} 0 1 {ty}");
            let _ = writeln!(out, "box 0 0 0 0");
            let _ = writeln!(out, "<< end >>");
        }
        let _ = writeln!(out, "<< end >>");
    }
    out
}

pub fn read_mag_str(text: &str) -> Result<Library> {
    read_mag_named(text, "MAG_TOP")
}

pub fn read_mag_named(text: &str, top_name: &str) -> Result<Library> {
    let lib = Library::new("magic", 1000);
    let mut cb = CellBuilder::new(CellName::new(top_name.to_string()));
    let mut current_layer: Option<klayout_core::LayerIndex> = None;
    let mut layers_for_name: HashMap<String, klayout_core::LayerIndex> = HashMap::new();
    let next_layer: u16 = 0;
    let mut child_cells: HashMap<String, CellId> = HashMap::new();

    let mut in_use = false;
    let mut use_name = String::new();
    let mut use_trans = Trans::IDENTITY;
    let _ = IoError::UnexpectedEof; // silence unused-import warning
    let _ = Bbox::EMPTY;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        match parts.first().copied() {
            Some("magic") | Some("tech") | Some("magscale") | Some("timestamp")
            | Some("string") => {
                // Header / metadata — ignored in v1.
            }
            Some("<<") => {
                // << layer >> or << end >>
                if parts.len() >= 3 && parts[2] == ">>" {
                    let lname = parts[1];
                    if lname == "end" {
                        current_layer = None;
                    } else {
                        // Synthetic per-name GDS number so distinct
                        // MAG layer names don't collapse onto a single
                        // (gds, dt) key inside the library's layer arena.
                        let _ = next_layer;
                        let li = *layers_for_name.entry(lname.to_string()).or_insert_with(|| {
                            let mut h: u32 = 0x811c9dc5;
                            for b in lname.bytes() {
                                h ^= b as u32;
                                h = h.wrapping_mul(0x0100_0193);
                            }
                            let synth = ((h & 0x7FFF) as u16).max(1);
                            // Use `dt = u16::MAX` so canonical-dump
                            // continues to recognize this as a "named
                            // layer with no GDS mapping" (which renders
                            // as -1 in the parity oracle).
                            lib.layer(LayerInfo::named(lname, synth, u16::MAX))
                        });
                        current_layer = Some(li);
                    }
                }
            }
            Some("rect") if parts.len() == 5 => {
                // Magic coords are in lambda units (default 1 lambda = 1 μm
                // unless `magscale` says otherwise). Library DBU is 1 nm,
                // so scale by 1000. Matches KLayout's reader.
                const MAG_TO_DBU: i64 = 1000;
                let xll: i64 = parts[1].parse::<i64>().unwrap_or(0) * MAG_TO_DBU;
                let yll: i64 = parts[2].parse::<i64>().unwrap_or(0) * MAG_TO_DBU;
                let xur: i64 = parts[3].parse::<i64>().unwrap_or(0) * MAG_TO_DBU;
                let yur: i64 = parts[4].parse::<i64>().unwrap_or(0) * MAG_TO_DBU;
                let bbox = Bbox::new(Point::new(xll, yll), Point::new(xur, yur));
                if let Some(li) = current_layer {
                    if !in_use {
                        // KLayout emits MAG `rect` lines as polygon shapes
                        // (4-vertex hull, CW from lowest-y/lowest-x). Match.
                        cb.add_shape(
                            li,
                            klayout_core::Polygon::from_hull(vec![
                                Point::new(bbox.min.x, bbox.min.y),
                                Point::new(bbox.min.x, bbox.max.y),
                                Point::new(bbox.max.x, bbox.max.y),
                                Point::new(bbox.max.x, bbox.min.y),
                            ]),
                        );
                    }
                }
            }
            Some("use") if parts.len() >= 2 => {
                in_use = true;
                use_name = parts[1].to_string();
                use_trans = Trans::IDENTITY;
            }
            Some("transform") if in_use && parts.len() >= 7 => {
                // 2D affine: a b tx c d ty (we read just the translation;
                // rotation/mirror would need full matrix decomposition).
                let tx: i64 = parts[3].parse().unwrap_or(0);
                let ty: i64 = parts[6].parse().unwrap_or(0);
                use_trans = Trans::translate(Vec2::new(tx, ty));
            }
            Some("box") if in_use && parts.len() == 5 => {
                // Bounding box of the use — informational, ignored in v1.
            }
            Some("end") if !in_use => {
                // Top-level cell end marker.
                break;
            }
            _ => {
                if in_use && line == "<< end >>" {
                    let child_id = *child_cells.entry(use_name.clone()).or_insert_with(|| {
                        let placeholder = CellBuilder::new(CellName::new(use_name.clone()));
                        lib.insert(placeholder)
                    });
                    cb.add_instance(klayout_core::Instance::new(child_id, use_trans));
                    in_use = false;
                    use_name.clear();
                }
            }
        }
    }

    lib.insert(cb);
    Ok(lib)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "magic\ntech sky130A\nmagscale 1 1\ntimestamp 0\n<< metal1 >>\nrect 0 0 100 50\nrect 200 0 300 50\n<< end >>\n<< metal2 >>\nrect 50 100 150 200\n<< end >>\n";

    #[test]
    fn reads_layers_and_rects() {
        let lib = read_mag_str(SAMPLE).unwrap();
        let top = lib.get(lib.by_name("MAG_TOP").unwrap());
        let layer_count = top.layers().count();
        assert_eq!(layer_count, 2);
    }

    #[test]
    fn empty_mag_yields_one_empty_cell() {
        let lib = read_mag_str("magic\ntech t\n<< end >>\n").unwrap();
        assert_eq!(lib.cell_count(), 1);
    }

    #[test]
    fn rects_distinguish_layers() {
        let lib = read_mag_str(SAMPLE).unwrap();
        let top = lib.get(lib.by_name("MAG_TOP").unwrap());
        let m1 = lib.layer_by_name("metal1").unwrap();
        let m2 = lib.layer_by_name("metal2").unwrap();
        let m1_count = top.shapes_on(m1).count();
        let m2_count = top.shapes_on(m2).count();
        assert_eq!(m1_count, 2);
        assert_eq!(m2_count, 1);
    }

    #[test]
    fn write_then_read_round_trips() {
        let lib1 = read_mag_str(SAMPLE).unwrap();
        let text = write_mag_str(&lib1, "sky130A");
        let lib2 = read_mag_str(&text).unwrap();
        assert_eq!(lib2.cell_count(), lib1.cell_count());
    }

    #[test]
    fn writer_emits_per_layer_blocks() {
        let lib = read_mag_str(SAMPLE).unwrap();
        let text = write_mag_str(&lib, "sky130A");
        assert!(text.contains("<< metal1 >>"));
        assert!(text.contains("<< metal2 >>"));
        assert!(text.contains("rect"));
    }
}
