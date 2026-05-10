//! Differential validation of klayout-rs against KLayout's `klayout.db`.

use klayout_core::{Cell, Library, Point, Polygon, Repetition, Rot4, Shape};
use serde_json::{json, Map, Value};

/// Flatten a polygon-with-holes into the keyhole-encoded hull KLayout
/// produces on read (rightmost-hole-vertex slit pointing right).
/// Mirror of `klayout-io`'s writer-side `make_keyhole_polygon`.
fn make_keyhole_dump(p: &Polygon) -> Vec<Point> {
    let mut hull: Vec<Point> = p.hull.iter().copied().collect();
    for hole in &p.holes {
        let hole_idx = match hole.iter().enumerate().max_by_key(|(_, pt)| (pt.x, pt.y)) {
            Some((i, _)) => i,
            None => continue,
        };
        let v_hole = hole[hole_idx];
        let mut best: Option<(usize, i64)> = None;
        for i in 0..hull.len() {
            let a = hull[i];
            let b = hull[(i + 1) % hull.len()];
            if a.x == b.x && a.x > v_hole.x {
                let y_lo = a.y.min(b.y);
                let y_hi = a.y.max(b.y);
                if y_lo <= v_hole.y && v_hole.y <= y_hi {
                    let cur_x = best.map(|(_, x)| x).unwrap_or(i64::MAX);
                    if a.x < cur_x {
                        best = Some((i, a.x));
                    }
                }
            }
        }
        let Some((edge_idx, x_cut)) = best else { continue };
        let v_outer = Point::new(x_cut, v_hole.y);
        let n = hole.len();
        let mut hole_cw: Vec<Point> = Vec::with_capacity(n);
        for k in 0..n {
            let idx = (hole_idx + n - k) % n;
            hole_cw.push(hole[idx]);
        }
        let mut new_hull: Vec<Point> = Vec::with_capacity(hull.len() + n + 3);
        new_hull.extend_from_slice(&hull[..=edge_idx]);
        new_hull.push(v_outer);
        new_hull.extend_from_slice(&hole_cw);
        new_hull.push(v_hole);
        new_hull.push(v_outer);
        new_hull.extend_from_slice(&hull[edge_idx + 1..]);
        hull = new_hull;
    }
    hull
}
use std::path::PathBuf;
use std::process::Command;

/// Absolute path to `validation/corpus/<rel>`.
pub fn corpus_path(rel: &str) -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest)
        .join("..")
        .join("corpus")
        .join(rel)
}

/// Absolute path to `validation/oracle.py`.
pub fn oracle_path() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest).join("..").join("oracle.py")
}

/// Locate a python interpreter that has `klayout.db` importable.
/// Honours `KLAYOUT_PYTHON` env var, otherwise uses the venv we set up
/// at `/tmp/klayout-venv/bin/python`. Returns `None` if neither exists.
pub fn klayout_python() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("KLAYOUT_PYTHON") {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
    }
    let default = PathBuf::from("/tmp/klayout-venv/bin/python");
    if default.exists() {
        Some(default)
    } else {
        None
    }
}

/// Read a GDS file via KLayout (subprocess) and return its canonical JSON dump.
/// Returns `None` if no KLayout-equipped Python is available — caller should
/// `skip!` the test in that case.
pub fn klayout_canonical_dump(gds: &std::path::Path) -> Option<Value> {
    let py = klayout_python()?;
    let output = Command::new(&py)
        .arg(oracle_path())
        .arg("verify")
        .arg(gds)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {e}", py.display()));
    if !output.status.success() {
        panic!(
            "oracle verify failed:\nstderr={}\nstdout={}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout),
        );
    }
    Some(serde_json::from_slice(&output.stdout).expect("oracle returned invalid JSON"))
}

// ---------- canonical Library dump (mirror of oracle.py canonical_layout_dump) ----------

pub fn canonical_dump(lib: &Library) -> Value {
    let cells: Vec<Value> = lib
        .all_cells()
        .iter()
        .map(|(_, c)| cell_to_json(c, lib))
        .collect();
    json!({
        "dbu_um": 1.0 / (lib.dbu() as f64),
        "cells": cells,
    })
}

fn cell_to_json(cell: &Cell, lib: &Library) -> Value {
    let mut shapes: Vec<Value> = Vec::new();
    for layer_idx in cell.layers() {
        let info = lib.layer_info(layer_idx);
        for s in cell.shapes_on(layer_idx) {
            shapes.push(shape_to_json(s, info.layer, info.datatype));
        }
    }
    shapes.sort_by_key(shape_sort_key);

    let mut instances: Vec<Value> = cell
        .instances()
        .iter()
        .map(|inst| {
            let child = lib.get(inst.cell);
            instance_to_json(inst, child.name().as_str())
        })
        .collect();
    instances.sort_by_key(sort_key);

    json!({
        "name": cell.name().as_str(),
        "shapes": shapes,
        "instances": instances,
    })
}

pub fn shape_to_json(shape: &Shape, layer: u16, datatype: u16) -> Value {
    // Sentinel: u16::MAX in *datatype* marks the whole layer as a
    // "named layer with no GDS mapping" — emit both fields as -1 to
    // match the KLayout `info.layer/-1, info.datatype/-1` convention
    // for non-GDS readers (CIF / MAG / DXF named layers, which carry
    // a hashed-from-name synthetic GDS number internally to keep the
    // layer-arena keys distinct).
    let named_marker = datatype == u16::MAX;
    let layer = if named_marker || layer == u16::MAX {
        -1i64
    } else {
        layer as i64
    };
    let datatype = if named_marker { -1i64 } else { datatype as i64 };
    match shape {
        Shape::Box(r) => json!({
            "type": "box", "layer": layer, "datatype": datatype,
            "left": r.bbox.min.x, "bottom": r.bbox.min.y,
            "right": r.bbox.max.x, "top": r.bbox.max.y,
        }),
        Shape::Polygon(p) => {
            // KLayout's `Region` stores polygons with separate hulls + holes,
            // but its GDS reader returns self-touching keyhole polygons
            // verbatim (no hole-detection on read). To match KLayout's view
            // here we flatten polygon-with-holes into the same keyhole hull.
            let hull_pts = if p.holes.is_empty() {
                p.hull.iter().copied().collect::<Vec<_>>()
            } else {
                make_keyhole_dump(p)
            };
            let hull: Vec<Value> = hull_pts
                .iter()
                .map(|pt| json!([pt.x, pt.y]))
                .collect();
            // After keyhole flattening, holes are encoded inside the hull.
            json!({
                "type": "polygon", "layer": layer, "datatype": datatype,
                "hull": hull, "holes": Vec::<Value>::new(),
            })
        }
        Shape::Path(p) => {
            let pts: Vec<Value> = p.points.iter().map(|pt| json!([pt.x, pt.y])).collect();
            // KLayout reports effective endpoint extensions: width/2 for
            // PATHTYPE 1 (round) and 2 (extended-square), user-set for
            // PATHTYPE 4 (custom). Match that derivation here.
            let half_width = p.width / 2;
            let (be, ee) = match p.cap {
                klayout_core::PathCap::Flat => (0, 0),
                klayout_core::PathCap::Round => (half_width, half_width),
                klayout_core::PathCap::Extended => {
                    if p.begin_ext != 0 || p.end_ext != 0 {
                        (p.begin_ext, p.end_ext)
                    } else {
                        (half_width, half_width)
                    }
                }
            };
            json!({
                "type": "path", "layer": layer, "datatype": datatype,
                "width": p.width, "points": pts,
                "begin_ext": be, "end_ext": ee,
                "round": matches!(p.cap, klayout_core::PathCap::Round),
            })
        }
        Shape::Text(t) => json!({
            "type": "text", "layer": layer, "datatype": datatype,
            "string": t.string.as_str(),
            "x": t.anchor.x, "y": t.anchor.y,
        }),
    }
}

pub fn rot_combined(t: klayout_core::Trans) -> i64 {
    let r = match t.rot {
        Rot4::R0 => 0,
        Rot4::R90 => 1,
        Rot4::R180 => 2,
        Rot4::R270 => 3,
    };
    if t.mirror { r + 4 } else { r }
}

pub fn instance_to_json(inst: &klayout_core::Instance, child_name: &str) -> Value {
    let mut obj = Map::new();
    obj.insert("sname".into(), json!(child_name));
    obj.insert(
        "trans".into(),
        json!([rot_combined(inst.trans), inst.trans.disp.x, inst.trans.disp.y]),
    );
    if let Some(Repetition::Regular { col, row, n_cols, n_rows }) = &inst.repetition {
        obj.insert(
            "array".into(),
            json!({
                "a": [row.x, row.y], "b": [col.x, col.y],
                "na": *n_rows, "nb": *n_cols,
            }),
        );
    }
    if !inst.properties.is_empty() {
        let mut props: Vec<(i64, String)> = Vec::new();
        for (k, v) in inst.properties.iter() {
            if let Ok(attr) = k.parse::<i64>() {
                let s = match v {
                    klayout_core::PropertyValue::String(s) => s.to_string(),
                    klayout_core::PropertyValue::Int(i) => i.to_string(),
                    _ => continue,
                };
                props.push((attr, s));
            }
        }
        props.sort_by_key(|(a, _)| *a);
        obj.insert(
            "properties".into(),
            Value::Array(
                props
                    .into_iter()
                    .map(|(a, s)| json!([a, s]))
                    .collect(),
            ),
        );
    }
    Value::Object(obj)
}

pub fn shape_sort_key(v: &Value) -> (i64, i64, String, String) {
    let layer = v.get("layer").and_then(|x| x.as_i64()).unwrap_or(0);
    let datatype = v.get("datatype").and_then(|x| x.as_i64()).unwrap_or(0);
    let typ = v.get("type").and_then(|x| x.as_str()).unwrap_or("").to_string();
    (layer, datatype, typ, sort_key(v))
}

pub fn sort_key(v: &Value) -> String {
    let mut s = Vec::new();
    write_canonical(&mut s, v);
    String::from_utf8(s).unwrap()
}

fn write_canonical(out: &mut Vec<u8>, v: &Value) {
    match v {
        Value::Null => out.extend_from_slice(b"null"),
        Value::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        Value::Number(n) => out.extend_from_slice(n.to_string().as_bytes()),
        Value::String(s) => {
            out.push(b'"');
            for c in s.chars() {
                match c {
                    '"' => out.extend_from_slice(b"\\\""),
                    '\\' => out.extend_from_slice(b"\\\\"),
                    '\n' => out.extend_from_slice(b"\\n"),
                    _ => {
                        let mut buf = [0u8; 4];
                        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
                    }
                }
            }
            out.push(b'"');
        }
        Value::Array(arr) => {
            out.push(b'[');
            for (i, item) in arr.iter().enumerate() {
                if i > 0 {
                    out.extend_from_slice(b", ");
                }
                write_canonical(out, item);
            }
            out.push(b']');
        }
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push(b'{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.extend_from_slice(b", ");
                }
                out.push(b'"');
                out.extend_from_slice(k.as_bytes());
                out.extend_from_slice(b"\": ");
                write_canonical(out, &map[*k]);
            }
            out.push(b'}');
        }
    }
}
