//! GDSII writer: `Library` → bytes.
//!
//! Deterministic output: cells are emitted in topological order (children
//! before parents), shapes within a cell in (layer-index, insertion) order.
//! Date stamps are zero. The same `Library` writes to byte-identical bytes
//! across runs and platforms.

use super::super::error::{IoError, Result};
use super::codec::*;
use super::records::*;
use klayout_core::{
    Cell, CellId, Instance, LayerIndex, Library, Path as CorePath, Point, Polygon, PropertyValue,
    Rect, Repetition, Rot4, Shape, Text, Trans,
};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

const GDS_VERSION: i16 = 600;

pub fn write_gds_path(lib: &Library, path: impl AsRef<Path>) -> Result<()> {
    let buf = write_gds_bytes(lib)?;
    std::fs::write(path.as_ref(), buf)?;
    Ok(())
}

pub fn write_gds_bytes(lib: &Library) -> Result<Vec<u8>> {
    let mut buf = Vec::new();

    write_record_i16(&mut buf, HEADER, &[GDS_VERSION]);
    write_record_i16(&mut buf, BGNLIB, &[0i16; 12]);
    let lib_name = if lib.name().is_empty() { "LIB" } else { lib.name() };
    write_record_str(&mut buf, LIBNAME, lib_name);

    // GDS UNITS: (user_unit_in_meters, dbu_in_meters_relative_to_user_unit?).
    // Convention used by KLayout/gdsfactory: UNITS records two f64s
    //   user_unit  = size of one user-unit in DBU (i.e. 1 / dbu_per_um when
    //                user-unit is 1 micron) — recorded as the ratio
    //                "DB unit / user unit".
    //   dbu_meters = size of one DB unit in meters.
    // Following that convention here: we treat the user unit as 1 micron, so
    //   user_unit  = 1 / dbu_per_um
    //   dbu_meters = 1e-6 / dbu_per_um
    let dbu_per_um = lib.dbu() as f64;
    let user_unit = 1.0 / dbu_per_um;
    // user_unit * 1e-6 is bit-exact for dbu=1000 (giving 1e-9). The naive
    // `1e-6 / dbu_per_um` differs by one ULP and KLayout's reader rejects it.
    let dbu_meters = user_unit * 1e-6;
    write_record_f64(&mut buf, UNITS, &[user_unit, dbu_meters]);

    // Topo sort cells (children before parents) so SREF references resolve.
    let cells = lib.all_cells();
    let id_to_idx: HashMap<CellId, usize> = cells
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, i))
        .collect();
    let order = topo_order(&cells, &id_to_idx)?;

    let id_to_name: HashMap<CellId, String> = cells
        .iter()
        .map(|(id, c)| (*id, c.name().as_str().to_string()))
        .collect();

    for idx in order {
        let (_id, cell) = &cells[idx];
        write_cell(&mut buf, cell, lib, &id_to_name)?;
    }

    write_record(&mut buf, ENDLIB, &[]);
    Ok(buf)
}

fn topo_order(
    cells: &[(CellId, Arc<Cell>)],
    id_to_idx: &HashMap<CellId, usize>,
) -> Result<Vec<usize>> {
    let mut visited = vec![false; cells.len()];
    let mut on_stack = vec![false; cells.len()];
    let mut order = Vec::with_capacity(cells.len());
    for i in 0..cells.len() {
        if !visited[i] {
            visit(i, cells, id_to_idx, &mut visited, &mut on_stack, &mut order)?;
        }
    }
    Ok(order)
}

fn visit(
    i: usize,
    cells: &[(CellId, Arc<Cell>)],
    id_to_idx: &HashMap<CellId, usize>,
    visited: &mut [bool],
    on_stack: &mut [bool],
    order: &mut Vec<usize>,
) -> Result<()> {
    if visited[i] {
        return Ok(());
    }
    if on_stack[i] {
        return Err(IoError::CycleDetected(
            cells[i].1.name().as_str().to_string(),
        ));
    }
    on_stack[i] = true;
    for inst in cells[i].1.instances() {
        if let Some(&j) = id_to_idx.get(&inst.cell) {
            visit(j, cells, id_to_idx, visited, on_stack, order)?;
        }
    }
    on_stack[i] = false;
    visited[i] = true;
    order.push(i);
    Ok(())
}

fn write_cell(
    buf: &mut Vec<u8>,
    cell: &Cell,
    lib: &Library,
    id_to_name: &HashMap<CellId, String>,
) -> Result<()> {
    write_record_i16(buf, BGNSTR, &[0i16; 12]);
    write_record_str(buf, STRNAME, cell.name().as_str());
    write_properties(buf, cell.properties());

    let layers: Vec<LayerIndex> = cell.layers().collect();
    for li in layers {
        let info = lib.layer_info(li);
        for shape in cell.shapes_on(li) {
            match shape {
                Shape::Polygon(p) => write_polygon(buf, info.layer, info.datatype, p),
                Shape::Path(p) => write_path(buf, info.layer, info.datatype, p),
                Shape::Box(r) => write_box(buf, info.layer, info.datatype, r),
                Shape::Text(t) => write_text(buf, info.layer, info.datatype, t),
            }
        }
    }

    for inst in cell.instances() {
        let name = id_to_name
            .get(&inst.cell)
            .ok_or_else(|| IoError::CellNotFound(format!("CellId({})", inst.cell.raw())))?;
        write_instance(buf, name, inst);
    }

    write_record(buf, ENDSTR, &[]);
    Ok(())
}

fn write_polygon(buf: &mut Vec<u8>, layer: u16, datatype: u16, p: &Polygon) {
    if p.hull.is_empty() {
        return;
    }
    let hull = if p.holes.is_empty() {
        p.hull.iter().copied().collect::<Vec<_>>()
    } else {
        // GDS BOUNDARY can't represent holes natively. Encode as a single
        // self-touching "keyhole" polygon — one continuous boundary that
        // visits the outer hull, branches via a zero-width slit into each
        // hole, walks the hole, returns through the slit, and continues.
        // KLayout's reader auto-detects this pattern and reconstructs
        // polygon-with-holes; downstream tools see an equivalent visual.
        make_keyhole_polygon(p)
    };
    write_record(buf, BOUNDARY, &[]);
    write_record_i16(buf, LAYER, &[layer as i16]);
    write_record_i16(buf, DATATYPE, &[datatype as i16]);
    let mut pts: Vec<i32> = Vec::with_capacity((hull.len() + 1) * 2);
    for pt in &hull {
        pts.push(pt.x as i32);
        pts.push(pt.y as i32);
    }
    let first = hull[0];
    pts.push(first.x as i32);
    pts.push(first.y as i32);
    write_record_i32(buf, XY, &pts);
    write_record(buf, ENDEL, &[]);
}

/// Keyhole-encode a polygon-with-holes as a single self-touching boundary.
///
/// For each hole, finds the rightmost hole vertex, casts a ray rightward to
/// the nearest vertical hull edge, and inserts a "slit" from that edge
/// point to the hole vertex. The slit traverses both directions — once
/// going in (cutting into the polygon to reach the hole), once coming
/// back out. Visually identical to the original polygon.
fn make_keyhole_polygon(p: &Polygon) -> Vec<Point> {
    let mut hull: Vec<Point> = p.hull.iter().copied().collect();
    for hole in &p.holes {
        let hole_idx = match hole
            .iter()
            .enumerate()
            .max_by_key(|(_, pt)| (pt.x, pt.y))
        {
            Some((i, _)) => i,
            None => continue,
        };
        let v_hole = hole[hole_idx];

        // Find the closest vertical hull edge crossing y=v_hole.y at x > v_hole.x.
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
        let Some((edge_idx, x_cut)) = best else {
            continue;
        };
        let v_outer = Point::new(x_cut, v_hole.y);

        // Walk the hole CW (reverse of stored CCW) starting from hole_idx.
        let n = hole.len();
        let mut hole_cw: Vec<Point> = Vec::with_capacity(n);
        for k in 0..n {
            let idx = (hole_idx + n - k) % n;
            hole_cw.push(hole[idx]);
        }
        // hole_cw = [v_hole, hole[i-1], ..., hole[i+1]]

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

fn write_path(buf: &mut Vec<u8>, layer: u16, datatype: u16, p: &CorePath) {
    write_record(buf, PATH, &[]);
    write_record_i16(buf, LAYER, &[layer as i16]);
    write_record_i16(buf, DATATYPE, &[datatype as i16]);
    let path_type: i16 = match p.cap {
        klayout_core::PathCap::Flat => 0,
        klayout_core::PathCap::Round => 1,
        klayout_core::PathCap::Extended => {
            if p.begin_ext != 0 || p.end_ext != 0 {
                4
            } else {
                2
            }
        }
    };
    write_record_i16(buf, PATHTYPE, &[path_type]);
    write_record_i32(buf, WIDTH, &[p.width as i32]);
    if path_type == 4 {
        write_record_i32(buf, BGNEXTN, &[p.begin_ext as i32]);
        write_record_i32(buf, ENDEXTN, &[p.end_ext as i32]);
    }
    let mut pts: Vec<i32> = Vec::with_capacity(p.points.len() * 2);
    for pt in &p.points {
        pts.push(pt.x as i32);
        pts.push(pt.y as i32);
    }
    write_record_i32(buf, XY, &pts);
    write_record(buf, ENDEL, &[]);
}

fn write_box(buf: &mut Vec<u8>, layer: u16, datatype: u16, r: &Rect) {
    write_record(buf, BOX_REC, &[]);
    write_record_i16(buf, LAYER, &[layer as i16]);
    write_record_i16(buf, BOXTYPE, &[datatype as i16]);
    let b = r.bbox;
    let pts = [
        (b.min.x as i32, b.min.y as i32),
        (b.max.x as i32, b.min.y as i32),
        (b.max.x as i32, b.max.y as i32),
        (b.min.x as i32, b.max.y as i32),
        (b.min.x as i32, b.min.y as i32),
    ];
    let mut flat = Vec::with_capacity(10);
    for (x, y) in pts {
        flat.push(x);
        flat.push(y);
    }
    write_record_i32(buf, XY, &flat);
    write_record(buf, ENDEL, &[]);
}

fn write_text(buf: &mut Vec<u8>, layer: u16, datatype: u16, t: &Text) {
    write_record(buf, TEXT, &[]);
    write_record_i16(buf, LAYER, &[layer as i16]);
    write_record_i16(buf, TEXTTYPE, &[datatype as i16]);
    write_record_i32(buf, XY, &[t.anchor.x as i32, t.anchor.y as i32]);
    write_record_str(buf, STRING, t.string.as_str());
    write_record(buf, ENDEL, &[]);
}

fn write_instance(buf: &mut Vec<u8>, sname: &str, inst: &Instance) {
    let is_array = inst.repetition.is_some();
    if is_array {
        write_record(buf, AREF, &[]);
    } else {
        write_record(buf, SREF, &[]);
    }
    write_record_str(buf, SNAME, sname);
    write_strans_block(buf, inst.trans);

    match &inst.repetition {
        Some(Repetition::Regular {
            row,
            col,
            n_rows,
            n_cols,
        }) => {
            let nc = (*n_cols).min(i16::MAX as u32) as i16;
            let nr = (*n_rows).min(i16::MAX as u32) as i16;
            write_record_i16(buf, COLROW, &[nc, nr]);
            let origin = inst.trans.disp;
            let col_end = Point::new(
                origin.x + col.x * (*n_cols as i64),
                origin.y + col.y * (*n_cols as i64),
            );
            let row_end = Point::new(
                origin.x + row.x * (*n_rows as i64),
                origin.y + row.y * (*n_rows as i64),
            );
            let pts = [
                (origin.x as i32, origin.y as i32),
                (col_end.x as i32, col_end.y as i32),
                (row_end.x as i32, row_end.y as i32),
            ];
            let mut flat = Vec::with_capacity(6);
            for (x, y) in pts {
                flat.push(x);
                flat.push(y);
            }
            write_record_i32(buf, XY, &flat);
        }
        Some(Repetition::Irregular { .. }) => {
            // GDS has no native irregular array. v1 emits the leader only.
            // A future writer could expand to N SREFs.
            write_record_i32(buf, XY, &[inst.trans.disp.x as i32, inst.trans.disp.y as i32]);
        }
        None => {
            write_record_i32(buf, XY, &[inst.trans.disp.x as i32, inst.trans.disp.y as i32]);
        }
    }

    write_properties(buf, &inst.properties);
    write_record(buf, ENDEL, &[]);
}

/// Emit any `Properties` entries whose key parses as a u16 PROPATTR id.
/// Non-numeric keys are silently skipped — GDS only carries numeric
/// attribute ids. Determinism: sorted by attr id.
fn write_properties(buf: &mut Vec<u8>, props: &klayout_core::Properties) {
    let mut typed: Vec<(u16, String)> = Vec::new();
    for (k, v) in props.iter() {
        if let Ok(attr) = k.parse::<u16>() {
            let s = match v {
                PropertyValue::String(s) => s.to_string(),
                PropertyValue::Int(i) => i.to_string(),
                PropertyValue::Float(f) => f.to_string(),
                PropertyValue::Bytes(_) | PropertyValue::List(_) => continue,
            };
            typed.push((attr, s));
        }
    }
    typed.sort_by_key(|(a, _)| *a);
    for (attr, value) in typed {
        write_record_i16(buf, PROPATTR, &[attr as i16]);
        write_record_str(buf, PROPVALUE, &value);
    }
}

fn write_strans_block(buf: &mut Vec<u8>, t: Trans) {
    let needs = t.mirror || t.rot != Rot4::R0;
    if !needs {
        return;
    }
    let strans_bits: u16 = if t.mirror { 0x8000 } else { 0 };
    write_record_bits(buf, STRANS, strans_bits);
    let angle = match t.rot {
        Rot4::R0 => 0.0,
        Rot4::R90 => 90.0,
        Rot4::R180 => 180.0,
        Rot4::R270 => 270.0,
    };
    if angle != 0.0 {
        write_record_f64(buf, ANGLE, &[angle]);
    }
}
