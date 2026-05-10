//! Streaming and partial-load GDS reader.
//!
//! The full reader (`read_gds_bytes`) materialises the entire library
//! in memory — fine for small to medium layouts but blows up on real
//! foundry files (tens of GB). [`GdsIndex`] scans a GDS once to build
//! a cell-name → byte-offset map, then offers two consumption modes:
//!
//! * **Streaming** — `for_each_shape(name, callback)` walks one cell's
//!   element records, invoking the callback per shape without
//!   constructing any owned data structures. The whole layout is never
//!   in memory.
//! * **Selective load** — `load_cells(names)` materialises the named
//!   cells and their dependencies (transitively reachable via SREF /
//!   AREF) into a regular `Library`. Useful for "I just want this one
//!   IP block" workflows.
//!
//! v1 indexes from a `&[u8]` (typically obtained via `mmap`). True
//! `Read`-based streaming over a network stream / pipe is a v2 — most
//! real EDA workflows mmap the file anyway.

use super::super::error::{IoError, Result};
use super::codec::*;
use super::records::*;
use klayout_core::{
    Bbox, CellBuilder, CellId, CellName, HAlign, Instance, LayerInfo, Library, Path as CorePath,
    PathCap, Point, Polygon, Rect, Repetition, Rot4, Text, Trans, VAlign, Vec2,
};
use std::collections::{HashMap, HashSet};

/// Index of cell offsets within a GDS byte buffer.
pub struct GdsIndex<'a> {
    bytes: &'a [u8],
    cells: HashMap<String, CellOffsets>,
    lib_name: String,
    dbu_meters: f64,
}

#[derive(Copy, Clone, Debug)]
struct CellOffsets {
    /// Position of the first record after BGNSTR/STRNAME (i.e., the first
    /// element record).
    body_start: usize,
    /// Position of the ENDSTR record (exclusive end of body).
    end_pos: usize,
}

/// One GDS element streamed out by `for_each_shape`. Strings reference
/// internal storage and are owned (cheap clones).
#[derive(Clone, Debug)]
pub enum StreamEvent {
    Boundary {
        layer: u16,
        datatype: u16,
        points: Vec<Point>,
    },
    Path {
        layer: u16,
        datatype: u16,
        width: i64,
        path_type: u16,
        points: Vec<Point>,
        bgn_ext: i64,
        end_ext: i64,
    },
    Box {
        layer: u16,
        datatype: u16,
        bbox: Bbox,
    },
    Text {
        layer: u16,
        texttype: u16,
        anchor: Point,
        string: String,
    },
    Sref {
        sname: String,
        trans: Trans,
    },
    Aref {
        sname: String,
        trans: Trans,
        n_cols: u16,
        n_rows: u16,
        col_step: Vec2,
        row_step: Vec2,
    },
}

/// Build an index by scanning the file once for cell offsets. Element
/// data is *not* parsed — only record headers are walked.
pub fn open_gds_bytes(bytes: &[u8]) -> Result<GdsIndex<'_>> {
    let mut r = ByteReader::new(bytes);
    let mut lib_name = String::new();
    let mut dbu_meters = 0.0;
    let mut cells: HashMap<String, CellOffsets> = HashMap::new();

    while let Some(h) = r.read_record_header()? {
        let data = r.take(h.data_len())?;
        match h.kind {
            HEADER | BGNLIB => {}
            LIBNAME => {
                lib_name = parse_string(data);
            }
            UNITS => {
                let f = parse_f64_array(data);
                if f.len() == 2 {
                    dbu_meters = f[1];
                }
            }
            BGNSTR => {
                // Next record is STRNAME.
                let strname_h = r
                    .read_record_header()?
                    .ok_or(IoError::UnexpectedEof)?;
                if strname_h.kind != STRNAME {
                    return Err(IoError::UnexpectedRecord {
                        kind: strname_h.kind,
                        context: "after BGNSTR (streaming)",
                    });
                }
                let name = parse_string(r.take(strname_h.data_len())?);
                let body_start = r.pos();
                // Walk records until ENDSTR.
                let mut depth_endstr_pos = None;
                while let Some(eh) = r.read_record_header()? {
                    let _ = r.take(eh.data_len())?;
                    if eh.kind == ENDSTR {
                        depth_endstr_pos = Some(r.pos() - eh.total_len as usize);
                        break;
                    }
                }
                let end_pos = depth_endstr_pos.ok_or(IoError::MissingRecord("ENDSTR"))?;
                cells.insert(
                    name,
                    CellOffsets {
                        body_start,
                        end_pos,
                    },
                );
            }
            ENDLIB => {
                return Ok(GdsIndex {
                    bytes,
                    cells,
                    lib_name,
                    dbu_meters,
                });
            }
            _ => {
                // Tolerate other library-level records.
            }
        }
    }
    Err(IoError::MissingRecord("ENDLIB"))
}

impl<'a> GdsIndex<'a> {
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    pub fn cell_names(&self) -> impl Iterator<Item = &str> {
        self.cells.keys().map(|s| s.as_str())
    }

    pub fn has_cell(&self, name: &str) -> bool {
        self.cells.contains_key(name)
    }

    pub fn lib_name(&self) -> &str {
        &self.lib_name
    }

    pub fn dbu(&self) -> i64 {
        if self.dbu_meters > 0.0 {
            let raw = (1e-6 / self.dbu_meters).round();
            if raw >= 1.0 && raw.is_finite() {
                raw as i64
            } else {
                1000
            }
        } else {
            1000
        }
    }

    /// Iterate every shape and instance in `cell`. The callback is
    /// invoked once per element. Cells referenced by SREF/AREF are
    /// **not** descended — the callback sees the reference only.
    pub fn for_each_shape<F>(&self, cell: &str, mut f: F) -> Result<()>
    where
        F: FnMut(StreamEvent),
    {
        let off = self
            .cells
            .get(cell)
            .ok_or_else(|| IoError::CellNotFound(cell.to_string()))?;
        let mut r = ByteReader::new(self.bytes);
        r.seek(off.body_start);
        while r.pos() < off.end_pos {
            let h = r
                .read_record_header()?
                .ok_or(IoError::UnexpectedEof)?;
            match h.kind {
                BOUNDARY => {
                    let _ = r.take(h.data_len())?;
                    let el = parse_boundary_stream(&mut r)?;
                    f(el);
                }
                PATH => {
                    let _ = r.take(h.data_len())?;
                    let el = parse_path_stream(&mut r)?;
                    f(el);
                }
                BOX_REC => {
                    let _ = r.take(h.data_len())?;
                    let el = parse_box_stream(&mut r)?;
                    f(el);
                }
                SREF => {
                    let _ = r.take(h.data_len())?;
                    let el = parse_sref_stream(&mut r)?;
                    f(el);
                }
                AREF => {
                    let _ = r.take(h.data_len())?;
                    let el = parse_aref_stream(&mut r)?;
                    f(el);
                }
                TEXT => {
                    let _ = r.take(h.data_len())?;
                    let el = parse_text_stream(&mut r)?;
                    f(el);
                }
                ENDSTR => {
                    return Ok(());
                }
                _ => {
                    // Skip unknown / cell-level property records.
                    let _ = r.take(h.data_len())?;
                }
            }
        }
        Ok(())
    }

    /// Materialise a subset of the GDS as a `Library`. The named cells
    /// plus any cells transitively reachable via SREF/AREF are loaded;
    /// everything else is skipped.
    pub fn load_cells(&self, names: &[&str]) -> Result<Library> {
        let mut wanted: HashSet<String> = HashSet::new();
        let mut queue: Vec<String> = names.iter().map(|s| s.to_string()).collect();
        while let Some(n) = queue.pop() {
            if !wanted.insert(n.clone()) {
                continue;
            }
            if !self.cells.contains_key(&n) {
                continue;
            }
            // Walk this cell's elements to find child references.
            self.for_each_shape(&n, |ev| match ev {
                StreamEvent::Sref { sname, .. } | StreamEvent::Aref { sname, .. } => {
                    queue.push(sname);
                }
                _ => {}
            })?;
        }

        // Build a Library by inserting cells in dependency order. We
        // already know `wanted`; we just need a topo order.
        let lib = Library::new(self.lib_name.clone(), self.dbu());
        let order = self.topo_order(&wanted)?;
        let mut name_to_id: HashMap<String, CellId> = HashMap::new();
        for cell_name in order {
            if !wanted.contains(&cell_name) || !self.cells.contains_key(&cell_name) {
                continue;
            }
            let mut cb = CellBuilder::new(CellName::new(cell_name.clone()));
            self.for_each_shape(&cell_name, |ev| {
                apply_event_to_builder(&lib, &mut cb, ev, &name_to_id);
            })?;
            let id = lib.insert(cb);
            name_to_id.insert(cell_name, id);
        }
        Ok(lib)
    }

    /// Topo order over `wanted` based on actual GDS dependency edges.
    fn topo_order(&self, wanted: &HashSet<String>) -> Result<Vec<String>> {
        let mut deps: HashMap<String, Vec<String>> = HashMap::new();
        for name in wanted {
            let mut children: Vec<String> = Vec::new();
            self.for_each_shape(name, |ev| match ev {
                StreamEvent::Sref { sname, .. } | StreamEvent::Aref { sname, .. }
                    if wanted.contains(&sname) =>
                {
                    children.push(sname);
                }
                _ => {}
            })?;
            deps.insert(name.clone(), children);
        }
        let mut visited: HashSet<String> = HashSet::new();
        let mut on_stack: HashSet<String> = HashSet::new();
        let mut order: Vec<String> = Vec::new();
        for name in wanted {
            visit_topo(name, &deps, &mut visited, &mut on_stack, &mut order)?;
        }
        Ok(order)
    }
}

fn visit_topo(
    name: &str,
    deps: &HashMap<String, Vec<String>>,
    visited: &mut HashSet<String>,
    on_stack: &mut HashSet<String>,
    order: &mut Vec<String>,
) -> Result<()> {
    if visited.contains(name) {
        return Ok(());
    }
    if !on_stack.insert(name.to_string()) {
        return Err(IoError::CycleDetected(name.to_string()));
    }
    if let Some(children) = deps.get(name) {
        for c in children {
            visit_topo(c, deps, visited, on_stack, order)?;
        }
    }
    on_stack.remove(name);
    visited.insert(name.to_string());
    order.push(name.to_string());
    Ok(())
}

fn apply_event_to_builder(
    lib: &Library,
    cb: &mut CellBuilder,
    ev: StreamEvent,
    name_to_id: &HashMap<String, CellId>,
) {
    match ev {
        StreamEvent::Boundary {
            layer,
            datatype,
            points,
        } => {
            let li = lib.layer(LayerInfo::gds(layer, datatype));
            let mut hull = points;
            if hull.len() >= 2 && hull.first() == hull.last() {
                hull.pop();
            }
            // Detect axis-aligned 4-vertex → Box.
            if let Some(r) = polygon_as_rect(&hull) {
                cb.add_shape(li, r);
            } else {
                cb.add_shape(li, Polygon::from_hull(hull));
            }
        }
        StreamEvent::Path {
            layer,
            datatype,
            width,
            path_type,
            points,
            bgn_ext,
            end_ext,
        } => {
            let li = lib.layer(LayerInfo::gds(layer, datatype));
            let mut p = CorePath::new(points, width);
            p.cap = match path_type {
                1 => PathCap::Round,
                2 | 4 => PathCap::Extended,
                _ => PathCap::Flat,
            };
            p.begin_ext = bgn_ext;
            p.end_ext = end_ext;
            cb.add_shape(li, p);
        }
        StreamEvent::Box {
            layer,
            datatype,
            bbox,
        } => {
            let li = lib.layer(LayerInfo::gds(layer, datatype));
            cb.add_shape(li, Rect::new(bbox));
        }
        StreamEvent::Text {
            layer,
            texttype,
            anchor,
            string,
        } => {
            let li = lib.layer(LayerInfo::gds(layer, texttype));
            let mut t = Text::new(string, anchor);
            t.halign = HAlign::Left;
            t.valign = VAlign::Bottom;
            cb.add_shape(li, t);
        }
        StreamEvent::Sref { sname, trans } => {
            if let Some(&cid) = name_to_id.get(&sname) {
                cb.add_instance(Instance::new(cid, trans));
            }
        }
        StreamEvent::Aref {
            sname,
            trans,
            n_cols,
            n_rows,
            col_step,
            row_step,
        } => {
            if let Some(&cid) = name_to_id.get(&sname) {
                let mut inst = Instance::new(cid, trans);
                inst.repetition = Some(Repetition::Regular {
                    col: col_step,
                    row: row_step,
                    n_cols: n_cols as u32,
                    n_rows: n_rows as u32,
                });
                cb.add_instance(inst);
            }
        }
    }
}

fn polygon_as_rect(hull: &[Point]) -> Option<Rect> {
    if hull.len() != 4 {
        return None;
    }
    let mut nxs = [hull[0].x, hull[1].x, hull[2].x, hull[3].x];
    let mut nys = [hull[0].y, hull[1].y, hull[2].y, hull[3].y];
    nxs.sort();
    nys.sort();
    let x_min = nxs[0];
    let x_max = nxs[3];
    let y_min = nys[0];
    let y_max = nys[3];
    if nxs[0] == nxs[1] && nxs[2] == nxs[3] && nys[0] == nys[1] && nys[2] == nys[3] {
        Some(Rect::new(Bbox::new(
            Point::new(x_min, y_min),
            Point::new(x_max, y_max),
        )))
    } else {
        None
    }
}

// ----- Per-element record-stream parsers (lightweight) -----

#[derive(Default)]
struct ElHeaders {
    layer: u16,
    datatype: u16,
    width: i32,
    path_type: u16,
    bgn_ext: i32,
    end_ext: i32,
    xy: Vec<(i32, i32)>,
    sname: String,
    strans: u16,
    angle: f64,
    mag: f64,
    n_cols: u16,
    n_rows: u16,
    string: String,
    pending_propattr: Option<u16>,
    properties: Vec<(u16, String)>,
}

fn read_until_endel(r: &mut ByteReader<'_>) -> Result<ElHeaders> {
    let mut h = ElHeaders {
        mag: 1.0,
        ..ElHeaders::default()
    };
    loop {
        let rh = r.read_record_header()?.ok_or(IoError::UnexpectedEof)?;
        let data = r.take(rh.data_len())?;
        match rh.kind {
            ENDEL => return Ok(h),
            LAYER => {
                h.layer = parse_i16_array(data).first().copied().unwrap_or(0) as u16;
            }
            DATATYPE | BOXTYPE | TEXTTYPE | NODETYPE => {
                h.datatype = parse_i16_array(data).first().copied().unwrap_or(0) as u16;
            }
            WIDTH => {
                h.width = parse_i32_array(data).first().copied().unwrap_or(0);
            }
            PATHTYPE => {
                h.path_type = parse_i16_array(data).first().copied().unwrap_or(0) as u16;
            }
            BGNEXTN => {
                h.bgn_ext = parse_i32_array(data).first().copied().unwrap_or(0);
            }
            ENDEXTN => {
                h.end_ext = parse_i32_array(data).first().copied().unwrap_or(0);
            }
            XY => {
                if data.len() % 8 != 0 {
                    return Err(IoError::BadXy { got: data.len() });
                }
                h.xy = data
                    .chunks_exact(8)
                    .map(|c| {
                        let x = i32::from_be_bytes([c[0], c[1], c[2], c[3]]);
                        let y = i32::from_be_bytes([c[4], c[5], c[6], c[7]]);
                        (x, y)
                    })
                    .collect();
            }
            SNAME => {
                h.sname = parse_string(data);
            }
            STRANS => {
                h.strans = u16::from_be_bytes([data[0], data[1]]);
            }
            ANGLE => {
                h.angle = parse_f64_array(data).first().copied().unwrap_or(0.0);
            }
            MAG => {
                h.mag = parse_f64_array(data).first().copied().unwrap_or(1.0);
            }
            COLROW => {
                let v = parse_i16_array(data);
                if v.len() == 2 {
                    h.n_cols = v[0] as u16;
                    h.n_rows = v[1] as u16;
                }
            }
            STRING => {
                h.string = parse_string(data);
            }
            PROPATTR => {
                h.pending_propattr = parse_i16_array(data).first().map(|v| *v as u16);
            }
            PROPVALUE => {
                if let Some(attr) = h.pending_propattr.take() {
                    h.properties.push((attr, parse_string(data)));
                }
            }
            _ => {}
        }
    }
}

fn parse_boundary_stream(r: &mut ByteReader<'_>) -> Result<StreamEvent> {
    let h = read_until_endel(r)?;
    let points: Vec<Point> = h
        .xy
        .iter()
        .map(|(x, y)| Point::new(*x as i64, *y as i64))
        .collect();
    Ok(StreamEvent::Boundary {
        layer: h.layer,
        datatype: h.datatype,
        points,
    })
}

fn parse_path_stream(r: &mut ByteReader<'_>) -> Result<StreamEvent> {
    let h = read_until_endel(r)?;
    let points: Vec<Point> = h
        .xy
        .iter()
        .map(|(x, y)| Point::new(*x as i64, *y as i64))
        .collect();
    Ok(StreamEvent::Path {
        layer: h.layer,
        datatype: h.datatype,
        width: h.width as i64,
        path_type: h.path_type,
        points,
        bgn_ext: h.bgn_ext as i64,
        end_ext: h.end_ext as i64,
    })
}

fn parse_box_stream(r: &mut ByteReader<'_>) -> Result<StreamEvent> {
    let h = read_until_endel(r)?;
    let xs: Vec<i64> = h.xy.iter().map(|(x, _)| *x as i64).collect();
    let ys: Vec<i64> = h.xy.iter().map(|(_, y)| *y as i64).collect();
    let bbox = Bbox::new(
        Point::new(*xs.iter().min().unwrap_or(&0), *ys.iter().min().unwrap_or(&0)),
        Point::new(*xs.iter().max().unwrap_or(&0), *ys.iter().max().unwrap_or(&0)),
    );
    Ok(StreamEvent::Box {
        layer: h.layer,
        datatype: h.datatype,
        bbox,
    })
}

fn parse_text_stream(r: &mut ByteReader<'_>) -> Result<StreamEvent> {
    let h = read_until_endel(r)?;
    let pos = h.xy.first().copied().unwrap_or((0, 0));
    Ok(StreamEvent::Text {
        layer: h.layer,
        texttype: h.datatype,
        anchor: Point::new(pos.0 as i64, pos.1 as i64),
        string: h.string,
    })
}

fn parse_sref_stream(r: &mut ByteReader<'_>) -> Result<StreamEvent> {
    let h = read_until_endel(r)?;
    let origin = h.xy.first().copied().unwrap_or((0, 0));
    let trans = strans_to_trans(h.strans, h.angle, origin)?;
    let _ = h.mag;
    let _: Vec<(u16, String)> = h.properties; // ignore properties on stream
    let _ = h.pending_propattr;
    Ok(StreamEvent::Sref {
        sname: h.sname,
        trans,
    })
}

fn parse_aref_stream(r: &mut ByteReader<'_>) -> Result<StreamEvent> {
    let h = read_until_endel(r)?;
    if h.xy.len() < 3 || h.n_cols == 0 || h.n_rows == 0 {
        return Err(IoError::ZeroArefCount);
    }
    let origin = h.xy[0];
    let col_end = h.xy[1];
    let row_end = h.xy[2];
    let trans = strans_to_trans(h.strans, h.angle, origin)?;
    // Step vectors per KLayout's AREF convention.
    let col_step = Vec2::new(
        (col_end.0 as i64 - origin.0 as i64) / (h.n_cols as i64).max(1),
        (col_end.1 as i64 - origin.1 as i64) / (h.n_cols as i64).max(1),
    );
    let row_step = Vec2::new(
        (row_end.0 as i64 - origin.0 as i64) / (h.n_rows as i64).max(1),
        (row_end.1 as i64 - origin.1 as i64) / (h.n_rows as i64).max(1),
    );
    Ok(StreamEvent::Aref {
        sname: h.sname,
        trans,
        n_cols: h.n_cols,
        n_rows: h.n_rows,
        col_step,
        row_step,
    })
}

fn strans_to_trans(strans: u16, angle: f64, origin: (i32, i32)) -> Result<Trans> {
    let mirror = (strans & 0x8000) != 0;
    let normalized = ((angle % 360.0) + 360.0) % 360.0;
    let rot = if (normalized - 0.0).abs() < 1e-6 || (normalized - 360.0).abs() < 1e-6 {
        Rot4::R0
    } else if (normalized - 90.0).abs() < 1e-6 {
        Rot4::R90
    } else if (normalized - 180.0).abs() < 1e-6 {
        Rot4::R180
    } else if (normalized - 270.0).abs() < 1e-6 {
        Rot4::R270
    } else {
        return Err(IoError::NonOrthogonalAngle(angle));
    };
    Ok(Trans::new(
        rot,
        mirror,
        Vec2::new(origin.0 as i64, origin.1 as i64),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{read_gds_bytes, write_gds_bytes};
    use klayout_core::{
        Bbox as Bb, CellBuilder, Instance, LayerInfo, Library, Point as P, Rect, Trans, Vec2,
    };

    fn build_test_lib() -> Library {
        let lib = Library::new("test", 1000);
        let l = lib.layer(LayerInfo::gds(1, 0));
        let mut cb = CellBuilder::new("a");
        cb.add_shape(l, Rect::new(Bb::new(P::new(0, 0), P::new(10, 10))));
        let aid = lib.insert(cb);
        let mut cb = CellBuilder::new("b");
        cb.add_shape(l, Rect::new(Bb::new(P::new(20, 0), P::new(30, 10))));
        cb.add_instance(Instance::new(aid, Trans::translate(Vec2::new(50, 0))));
        lib.insert(cb);
        // Independent cell.
        let mut cb = CellBuilder::new("c");
        cb.add_shape(l, Rect::new(Bb::new(P::new(0, 0), P::new(5, 5))));
        lib.insert(cb);
        lib
    }

    #[test]
    fn index_lists_all_cells() {
        let lib = build_test_lib();
        let bytes = write_gds_bytes(&lib).unwrap();
        let idx = open_gds_bytes(&bytes).unwrap();
        assert_eq!(idx.cell_count(), 3);
        let names: HashSet<&str> = idx.cell_names().collect();
        assert!(names.contains("a"));
        assert!(names.contains("b"));
        assert!(names.contains("c"));
    }

    #[test]
    fn for_each_shape_emits_each_element() {
        let lib = build_test_lib();
        let bytes = write_gds_bytes(&lib).unwrap();
        let idx = open_gds_bytes(&bytes).unwrap();
        let mut shape_count = 0;
        let mut inst_count = 0;
        idx.for_each_shape("b", |ev| match ev {
            StreamEvent::Boundary { .. } | StreamEvent::Box { .. } => shape_count += 1,
            StreamEvent::Sref { .. } | StreamEvent::Aref { .. } => inst_count += 1,
            _ => {}
        })
        .unwrap();
        assert_eq!(shape_count, 1);
        assert_eq!(inst_count, 1);
    }

    #[test]
    fn selective_load_only_pulls_dependencies() {
        let lib = build_test_lib();
        let bytes = write_gds_bytes(&lib).unwrap();
        let idx = open_gds_bytes(&bytes).unwrap();
        let partial = idx.load_cells(&["b"]).unwrap();
        // "b" depends on "a" → both load. "c" should not.
        assert_eq!(partial.cell_count(), 2);
        assert!(partial.by_name("a").is_some());
        assert!(partial.by_name("b").is_some());
        assert!(partial.by_name("c").is_none());
    }

    #[test]
    fn selective_load_isolated_cell_only() {
        let lib = build_test_lib();
        let bytes = write_gds_bytes(&lib).unwrap();
        let idx = open_gds_bytes(&bytes).unwrap();
        let partial = idx.load_cells(&["c"]).unwrap();
        assert_eq!(partial.cell_count(), 1);
        assert!(partial.by_name("c").is_some());
    }

    #[test]
    fn selective_load_round_trips_shapes() {
        let lib = build_test_lib();
        let bytes = write_gds_bytes(&lib).unwrap();
        let idx = open_gds_bytes(&bytes).unwrap();
        let partial = idx.load_cells(&["a"]).unwrap();
        let original = read_gds_bytes(&bytes).unwrap();
        assert_eq!(
            partial.get(partial.by_name("a").unwrap()).content_hash(),
            original.get(original.by_name("a").unwrap()).content_hash()
        );
    }
}
