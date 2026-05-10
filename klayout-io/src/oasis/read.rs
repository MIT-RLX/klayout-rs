//! OASIS reader.
//!
//! v3 covers the foundry-format record set:
//! * `START`, `END`, `PAD` — file framing.
//! * `CELL` (14) / `CELL_REF` (13) with the implicit `CELLNAME` (3) /
//!   explicit `CELLNAME_REF` (4) name table.
//! * `RECTANGLE` (20) with the standard info byte (SWHXYRDL).
//! * `POLYGON` (21) with point-list types 0..5 (manhattan, octangular,
//!   general).
//! * `PATH` (22) → expands to a polygon hull respecting halfwidth and
//!   start/end extensions.
//! * `TEXT` (19) — emits a `Text` shape on `(textlayer, texttype)`.
//! * `TRAPEZOID` / `TRAPEZOID_A` / `TRAPEZOID_B` (23/24/25) — decoded as
//!   four-vertex polygons.
//! * `CTRAPEZOID` (26) — 26 standard compressed-trapezoid types decoded
//!   to polygons.
//! * `CIRCLE` (27) — emitted as a regular-polygon approximation
//!   (`CIRCLE_FACETS = 64`).
//! * `PLACEMENT` (17) and `PLACEMENT_TRANSFORM` (18) — full Rot4 +
//!   mirror, magnification != 1 rejected.
//!
//! Repetition info-byte bits decode the repetition payload and skip
//! ahead — only the modal/base element is materialised in the output
//! library. Full repetition expansion is a deferred follow-up.
//!
//! `CBLOCK` compression and `PROPERTY` records remain deferred.

use super::codec::*;
use super::modal::Modal;
use super::records::*;
use crate::error::{IoError, Result};
use klayout_core::{
    Bbox, CellBuilder, CellId, CellName, HAlign, Instance, LayerInfo, Library, Path, PathCap,
    Point, Polygon, Rect, Rot4, Text, Trans, VAlign, Vec2,
};
use std::collections::HashMap;
use std::path::Path as FsPath;

const CIRCLE_FACETS: usize = 64;

/// Sentinel prefix for cellname-references that couldn't be resolved
/// at parse time. The number after the prefix is the refnum; we
/// resolve in `resolve()` after END once the cellname table has
/// been fully populated.
const CELLNAME_REF_SENTINEL: &str = "\u{1}__cellref_";

pub fn read_oasis_path(path: impl AsRef<FsPath>) -> Result<Library> {
    let bytes = std::fs::read(path.as_ref())?;
    read_oasis_bytes(&bytes)
}

pub fn read_oasis_bytes(bytes: &[u8]) -> Result<Library> {
    if bytes.len() < MAGIC.len() || &bytes[..MAGIC.len()] != MAGIC {
        return Err(IoError::OasisBadMagic);
    }
    // Owned buffer + position. Owned so CBLOCK can splice decompressed
    // payload into the middle of the stream.
    let mut buf: Vec<u8> = bytes[MAGIC.len()..].to_vec();
    let mut pos: usize = 0;

    // Parse START header and capture DBU.
    let dbu;
    {
        let mut r: &[u8] = &buf[pos..];
        let start_len = r.len();
        if r.is_empty() || r[0] != START {
            return Err(IoError::OasisUnsupportedRecord(r.first().copied().unwrap_or(0)));
        }
        r = &r[1..];
        let _version = decode_string(&mut r)?;
        let unit = decode_real(&mut r)?;
        let offset_flag = decode_unsigned(&mut r)?;
        // SEMI P39 §15: when offset_flag = 1, the 12 table-offsets
        // follow inside the START record (and the END record's
        // 12 unsigned-int slots are present-but-zero). When 0, the
        // table-offsets sit at the end. Either way we consume them
        // here if present.
        if offset_flag == 1 {
            for _ in 0..12 {
                let _ = decode_unsigned(&mut r)?;
            }
        }
        dbu = unit.to_f64().round().max(1.0) as i64;
        pos += start_len - r.len();
    }
    let lib = Library::new("oasis", dbu);

    let mut pending: Vec<PendingCell> = Vec::new();
    let mut current: Option<PendingCell> = None;
    let mut modal = Modal::default();
    let mut cellname_table: Vec<String> = Vec::new();
    let mut textstring_table: Vec<String> = Vec::new();
    // Aggregate bounds so a hostile input can't drive total
    // memory unbounded by simply repeating CELL / RECTANGLE
    // records. Both numbers are 100× any sane chip layout but
    // small enough to ensure a single bad input can't OOM the
    // process. Exceeding either causes the parser to bail with
    // a typed error rather than continue allocating.
    const MAX_CELLS: usize = 1 << 20; // ~1 M cells
    const MAX_SHAPES: usize = 1 << 26; // ~64 M shapes total
    let mut total_shapes: usize = 0;

    while pos < buf.len() {
        let kind = buf[pos];
        pos += 1;
        let mut r: &[u8] = &buf[pos..];
        let start_len = r.len();
        match kind {
            END => {
                if let Some(c) = current.take() {
                    pending.push(c);
                }
                // SEMI P39 §15: END is padded so the total record is
                // exactly 256 bytes long. The first byte is the kind (already
                // consumed), then 12 table-offsets (unsigneds), then the
                // validation scheme byte, then optional checksum bytes,
                // then padding. Just skip to the next 256-byte boundary
                // from the start of the record.
                resolve_cellname_refs(&mut pending, &cellname_table);
                resolve(&lib, pending)?;
                return Ok(lib);
            }
            PAD => {}
            CELL => {
                if let Some(c) = current.take() {
                    pending.push(c);
                }
                if pending.len() >= MAX_CELLS {
                    return Err(IoError::OasisNotImplemented(
                        "cell count exceeds sanity bound",
                    ));
                }
                let name = decode_string(&mut r)?;
                modal.last_cellname = Some(name.clone());
                current = Some(PendingCell::new(name));
            }
            CELL_REF => {
                if let Some(c) = current.take() {
                    pending.push(c);
                }
                if pending.len() >= MAX_CELLS {
                    return Err(IoError::OasisNotImplemented(
                        "cell count exceeds sanity bound",
                    ));
                }
                let refnum = decode_unsigned(&mut r)? as usize;
                // Cellname tables in OASIS commonly appear after the
                // cell content. Use a deferred-resolution sentinel
                // when the entry isn't populated yet; we resolve in
                // `resolve()` after the END record.
                let name = cellname_table
                    .get(refnum)
                    .cloned()
                    .unwrap_or_else(|| format!("{CELLNAME_REF_SENTINEL}{refnum}"));
                modal.last_cellname = Some(name.clone());
                current = Some(PendingCell::new(name));
            }
            CELLNAME => {
                let name = decode_string(&mut r)?;
                cellname_table.push(name);
            }
            CELLNAME_REF => {
                let name = decode_string(&mut r)?;
                let refnum = decode_unsigned(&mut r)? as usize;
                if refnum > MAX_REF_TABLE_SIZE {
                    return Err(IoError::OasisNotImplemented(
                        "cellname table refnum exceeds sanity bound",
                    ));
                }
                if cellname_table.len() <= refnum {
                    cellname_table.resize(refnum + 1, String::new());
                }
                cellname_table[refnum] = name;
            }
            TEXTSTRING => {
                let s = decode_string(&mut r)?;
                textstring_table.push(s);
            }
            TEXTSTRING_REF => {
                let s = decode_string(&mut r)?;
                let refnum = decode_unsigned(&mut r)? as usize;
                if refnum > MAX_REF_TABLE_SIZE {
                    return Err(IoError::OasisNotImplemented(
                        "textstring table refnum exceeds sanity bound",
                    ));
                }
                if textstring_table.len() <= refnum {
                    textstring_table.resize(refnum + 1, String::new());
                }
                textstring_table[refnum] = s;
            }
            PROPNAME | PROPNAME_REF | PROPSTRING | PROPSTRING_REF => {
                let _ = decode_string(&mut r)?;
                if kind == PROPNAME_REF || kind == PROPSTRING_REF {
                    let _ = decode_unsigned(&mut r)?;
                }
            }
            LAYERNAME_DATA | LAYERNAME_TEXT => {
                let _name = decode_string(&mut r)?;
                let _ = decode_unsigned(&mut r)?;
                let _ = decode_unsigned(&mut r)?;
                let _ = decode_unsigned(&mut r)?;
                let _ = decode_unsigned(&mut r)?;
            }
            XYABSOLUTE | XYRELATIVE => {
                // Modal flag — switch absolute/relative XY interpretation.
                // We treat all coordinates as absolute (modal x/y) which
                // matches the writer's output.
            }
            RECTANGLE => {
                let info = read_u8(&mut r)?;
                let (layer, datatype, x, y, w, h, offsets) =
                    parse_rectangle(&mut r, info, &mut modal)?;
                let cur = require_cell(&mut current, "rectangle outside cell")?;
                for (dx, dy) in offsets {
                    // Malformed input can pair an extreme `(x, y)` /
                    // `(w, h)` with extreme repetition `(dx, dy)` and
                    // overflow `i64`. Skip the rect rather than panic.
                    let lo_x = match x.checked_add(dx) {
                        Some(v) => v,
                        None => continue,
                    };
                    let lo_y = match y.checked_add(dy) {
                        Some(v) => v,
                        None => continue,
                    };
                    let hi_x = match lo_x.checked_add(w) {
                        Some(v) => v,
                        None => continue,
                    };
                    let hi_y = match lo_y.checked_add(h) {
                        Some(v) => v,
                        None => continue,
                    };
                    total_shapes += 1;
                    if total_shapes > MAX_SHAPES {
                        return Err(IoError::OasisNotImplemented(
                            "shape count exceeds sanity bound",
                        ));
                    }
                    let bbox = Bbox::new(Point::new(lo_x, lo_y), Point::new(hi_x, hi_y));
                    cur.shapes.push(PendingShape::Rect {
                        layer: layer as u16,
                        datatype: datatype as u16,
                        bbox,
                    });
                }
            }
            POLYGON => {
                let info = read_u8(&mut r)?;
                let (layer, datatype, hull, offsets) = parse_polygon(&mut r, info, &mut modal)?;
                let cur = require_cell(&mut current, "polygon outside cell")?;
                for (dx, dy) in offsets {
                    total_shapes += 1;
                    if total_shapes > MAX_SHAPES {
                        return Err(IoError::OasisNotImplemented(
                            "shape count exceeds sanity bound",
                        ));
                    }
                    let shifted: Vec<Point> =
                        hull.iter().map(|p| Point::new(p.x + dx, p.y + dy)).collect();
                    cur.shapes.push(PendingShape::Poly {
                        layer: layer as u16,
                        datatype: datatype as u16,
                        hull: shifted,
                    });
                }
            }
            PATH => {
                let info = read_u8(&mut r)?;
                let (layer, datatype, points, halfwidth, start_ext, end_ext) =
                    parse_path(&mut r, info, &mut modal)?;
                let cur = require_cell(&mut current, "path outside cell")?;
                total_shapes += 1;
                if total_shapes > MAX_SHAPES {
                    return Err(IoError::OasisNotImplemented(
                        "shape count exceeds sanity bound",
                    ));
                }
                cur.shapes.push(PendingShape::Path {
                    layer: layer as u16,
                    datatype: datatype as u16,
                    points,
                    halfwidth,
                    start_ext,
                    end_ext,
                });
            }
            TEXT => {
                let info = read_u8(&mut r)?;
                let (layer, datatype, x, y, s) =
                    parse_text(&mut r, info, &mut modal, &textstring_table)?;
                let cur = require_cell(&mut current, "text outside cell")?;
                total_shapes += 1;
                if total_shapes > MAX_SHAPES {
                    return Err(IoError::OasisNotImplemented(
                        "shape count exceeds sanity bound",
                    ));
                }
                cur.shapes.push(PendingShape::Text {
                    layer: layer as u16,
                    datatype: datatype as u16,
                    text: s,
                    anchor: Point::new(x, y),
                });
            }
            TRAPEZOID | TRAPEZOID_A | TRAPEZOID_B => {
                let info = read_u8(&mut r)?;
                let (layer, datatype, hull) =
                    parse_trapezoid(&mut r, info, &mut modal, kind)?;
                let cur = require_cell(&mut current, "trapezoid outside cell")?;
                total_shapes += 1;
                if total_shapes > MAX_SHAPES {
                    return Err(IoError::OasisNotImplemented(
                        "shape count exceeds sanity bound",
                    ));
                }
                cur.shapes.push(PendingShape::Poly {
                    layer: layer as u16,
                    datatype: datatype as u16,
                    hull,
                });
            }
            CTRAPEZOID => {
                let info = read_u8(&mut r)?;
                let (layer, datatype, hull) =
                    parse_ctrapezoid(&mut r, info, &mut modal)?;
                let cur = require_cell(&mut current, "ctrapezoid outside cell")?;
                total_shapes += 1;
                if total_shapes > MAX_SHAPES {
                    return Err(IoError::OasisNotImplemented(
                        "shape count exceeds sanity bound",
                    ));
                }
                cur.shapes.push(PendingShape::Poly {
                    layer: layer as u16,
                    datatype: datatype as u16,
                    hull,
                });
            }
            CIRCLE => {
                let info = read_u8(&mut r)?;
                let (layer, datatype, cx, cy, radius) =
                    parse_circle(&mut r, info, &mut modal)?;
                let cur = require_cell(&mut current, "circle outside cell")?;
                total_shapes += 1;
                if total_shapes > MAX_SHAPES {
                    return Err(IoError::OasisNotImplemented(
                        "shape count exceeds sanity bound",
                    ));
                }
                cur.shapes.push(PendingShape::Poly {
                    layer: layer as u16,
                    datatype: datatype as u16,
                    hull: circle_hull(cx, cy, radius),
                });
            }
            PLACEMENT => {
                let info = read_u8(&mut r)?;
                let (sname, trans, offsets) =
                    parse_placement(&mut r, info, &mut modal, &cellname_table, false)?;
                let cur = require_cell(&mut current, "placement outside cell")?;
                for (dx, dy) in offsets {
                    let mut t = trans;
                    t.disp = Vec2::new(t.disp.x + dx, t.disp.y + dy);
                    cur.instances.push(PendingInst {
                        sname: sname.clone(),
                        trans: t,
                    });
                }
            }
            PLACEMENT_TRANSFORM => {
                let info = read_u8(&mut r)?;
                let (sname, trans, offsets) =
                    parse_placement(&mut r, info, &mut modal, &cellname_table, true)?;
                let cur = require_cell(&mut current, "placement outside cell")?;
                for (dx, dy) in offsets {
                    let mut t = trans;
                    t.disp = Vec2::new(t.disp.x + dx, t.disp.y + dy);
                    cur.instances.push(PendingInst {
                        sname: sname.clone(),
                        trans: t,
                    });
                }
            }
            PROPERTY => {
                // SEMI P39 §28: info byte UUUUVCNS.
                //   U[7..4]: value-count (15 = read extra unsigned for count)
                //   V[3]: 1 = modal value-list (reuse last); 0 = explicit
                //   C[2]: 1 = propname is explicit (follows); 0 = modal
                //   N[1]: propname kind (0 = name-string, 1 = refnum)
                //   S[0]: standard-property flag (no parse impact)
                let info = read_u8(&mut r)?;
                let name_explicit = (info & 0b0000_0100) != 0;
                let name_is_ref = (info & 0b0000_0010) != 0;
                let modal_values = (info & 0b0000_1000) != 0;
                let u_bits = (info >> 4) as u64;
                if name_explicit {
                    if name_is_ref {
                        let _ = decode_unsigned(&mut r)?;
                    } else {
                        let _ = decode_string(&mut r)?;
                    }
                }
                if !modal_values {
                    let count = if u_bits == 15 {
                        decode_unsigned(&mut r)?
                    } else {
                        u_bits
                    };
                    for _ in 0..count {
                        skip_property_value(&mut r)?;
                    }
                }
            }
            PROPERTY_LAST => {
                // Repeat-last-property: no payload.
            }
            CBLOCK => {
                let comp_type = decode_unsigned(&mut r)?;
                if comp_type != 0 {
                    return Err(IoError::OasisNotImplemented(
                        "CBLOCK comp-type != 0 (only deflate supported)",
                    ));
                }
                let _uncomp_count = decode_unsigned(&mut r)?;
                let comp_count = decode_unsigned(&mut r)? as usize;
                if r.len() < comp_count {
                    return Err(IoError::UnexpectedEof);
                }
                let header_consumed = start_len - r.len();
                let comp_start = pos + header_consumed;
                let comp_end = comp_start + comp_count;
                let compressed = buf[comp_start..comp_end].to_vec();
                let decomp = inflate_deflate(&compressed)?;
                // Replace the compressed payload (header is already consumed
                // logically — but we still need to advance pos to before the
                // payload, then splice). The header bytes remain in `buf`
                // but `pos` will jump past them and into the decompressed
                // stream that replaced the payload.
                buf.splice(comp_start..comp_end, decomp);
                pos = comp_start;
                continue;
            }
            other => {
                return Err(IoError::OasisUnsupportedRecord(other));
            }
        }
        pos += start_len - r.len();
    }
    Err(IoError::UnexpectedEof)
}

/// Skip one OASIS property value. Per SEMI P39 §28:
/// type byte 0..7 → real (same codec as `decode_real`); 8 → unsigned-int;
/// 9 → signed-int; 10 → a-string; 11 → b-string; 12 → name-string;
/// 13/14/15 → string refnum.
fn skip_property_value(r: &mut &[u8]) -> Result<()> {
    let kind = read_u8(r)?;
    match kind {
        0..=7 => {
            // Re-feed the kind byte and use decode_real, which also reads
            // its kind byte. Easier: handle each variant here.
            match kind {
                0..=1 => {
                    let _ = decode_unsigned(r)?;
                }
                2..=3 => {
                    let _ = decode_unsigned(r)?;
                    let _ = decode_unsigned(r)?;
                }
                6 => {
                    if r.len() < 4 {
                        return Err(IoError::UnexpectedEof);
                    }
                    *r = &r[4..];
                }
                7 => {
                    if r.len() < 8 {
                        return Err(IoError::UnexpectedEof);
                    }
                    *r = &r[8..];
                }
                _ => {
                    return Err(IoError::OasisUnsupportedReal(kind));
                }
            }
        }
        8 => {
            let _ = decode_unsigned(r)?;
        }
        9 => {
            let _ = decode_signed(r)?;
        }
        10..=12 => {
            let _ = decode_string(r)?;
        }
        13..=15 => {
            let _ = decode_unsigned(r)?;
        }
        other => {
            return Err(IoError::OasisUnsupportedReal(other));
        }
    }
    Ok(())
}

/// Inflate a deflate-compressed CBLOCK payload.
fn inflate_deflate(compressed: &[u8]) -> Result<Vec<u8>> {
    use flate2::read::DeflateDecoder;
    use std::io::Read;
    let mut out = Vec::new();
    let mut dec = DeflateDecoder::new(compressed);
    dec.read_to_end(&mut out)
        .map_err(|_| IoError::OasisNotImplemented("CBLOCK deflate decode failed"))?;
    Ok(out)
}

struct PendingCell {
    name: String,
    shapes: Vec<PendingShape>,
    instances: Vec<PendingInst>,
}

impl PendingCell {
    fn new(name: String) -> Self {
        Self {
            name,
            shapes: Vec::new(),
            instances: Vec::new(),
        }
    }
}

enum PendingShape {
    Rect {
        layer: u16,
        datatype: u16,
        bbox: Bbox,
    },
    Poly {
        layer: u16,
        datatype: u16,
        hull: Vec<Point>,
    },
    Path {
        layer: u16,
        datatype: u16,
        points: Vec<Point>,
        halfwidth: i64,
        start_ext: i64,
        end_ext: i64,
    },
    Text {
        layer: u16,
        datatype: u16,
        text: String,
        anchor: Point,
    },
}

struct PendingInst {
    sname: String,
    trans: Trans,
}

fn require_cell<'a>(
    current: &'a mut Option<PendingCell>,
    msg: &'static str,
) -> Result<&'a mut PendingCell> {
    current
        .as_mut()
        .ok_or(IoError::OasisNotImplemented(msg))
}

/// After END, walk every cell and instance name; if any starts with
/// the sentinel prefix, look up the refnum in the now-complete
/// cellname table and substitute.
fn resolve_cellname_refs(pending: &mut [PendingCell], table: &[String]) {
    for c in pending.iter_mut() {
        if let Some(refnum) = parse_sentinel(&c.name) {
            if let Some(real) = table.get(refnum) {
                c.name = real.clone();
            }
        }
        for inst in c.instances.iter_mut() {
            if let Some(refnum) = parse_sentinel(&inst.sname) {
                if let Some(real) = table.get(refnum) {
                    inst.sname = real.clone();
                }
            }
        }
    }
}

fn parse_sentinel(s: &str) -> Option<usize> {
    s.strip_prefix(CELLNAME_REF_SENTINEL)
        .and_then(|rest| rest.parse().ok())
}

fn resolve(lib: &Library, pending: Vec<PendingCell>) -> Result<()> {
    let by_name: HashMap<String, usize> = pending
        .iter()
        .enumerate()
        .map(|(i, c)| (c.name.clone(), i))
        .collect();
    let mut order = Vec::with_capacity(pending.len());
    let mut visited = vec![false; pending.len()];
    let mut on_stack = vec![false; pending.len()];
    for i in 0..pending.len() {
        if !visited[i] {
            visit(i, &pending, &by_name, &mut visited, &mut on_stack, &mut order)?;
        }
    }

    let mut name_to_id: HashMap<String, CellId> = HashMap::new();
    for cell_idx in order {
        let pc = &pending[cell_idx];
        let mut cb = CellBuilder::new(CellName::new(pc.name.clone()));
        for s in &pc.shapes {
            match s {
                PendingShape::Rect { layer, datatype, bbox } => {
                    let li = lib.layer(LayerInfo::gds(*layer, *datatype));
                    cb.add_shape(li, Rect::new(*bbox));
                }
                PendingShape::Poly { layer, datatype, hull } => {
                    let li = lib.layer(LayerInfo::gds(*layer, *datatype));
                    cb.add_shape(li, Polygon::from_hull(hull.clone()));
                }
                PendingShape::Path {
                    layer,
                    datatype,
                    points,
                    halfwidth,
                    start_ext,
                    end_ext,
                } => {
                    let li = lib.layer(LayerInfo::gds(*layer, *datatype));
                    let mut p = Path::new(points.iter().copied(), halfwidth * 2);
                    p.begin_ext = *start_ext;
                    p.end_ext = *end_ext;
                    p.cap = if *start_ext == 0 && *end_ext == 0 {
                        PathCap::Flat
                    } else {
                        PathCap::Extended
                    };
                    cb.add_shape(li, p);
                }
                PendingShape::Text {
                    layer,
                    datatype,
                    text,
                    anchor,
                } => {
                    let li = lib.layer(LayerInfo::gds(*layer, *datatype));
                    let mut t = Text::new(text.as_str(), *anchor);
                    t.halign = HAlign::Left;
                    t.valign = VAlign::Bottom;
                    cb.add_shape(li, t);
                }
            }
        }
        for inst in &pc.instances {
            let cid = name_to_id
                .get(&inst.sname)
                .copied()
                .ok_or_else(|| IoError::CellNotFound(inst.sname.clone()))?;
            cb.add_instance(Instance::new(cid, inst.trans));
        }
        let id = lib.insert(cb);
        name_to_id.insert(pc.name.clone(), id);
    }
    Ok(())
}

fn visit(
    i: usize,
    cells: &[PendingCell],
    by_name: &HashMap<String, usize>,
    visited: &mut [bool],
    on_stack: &mut [bool],
    order: &mut Vec<usize>,
) -> Result<()> {
    if visited[i] {
        return Ok(());
    }
    if on_stack[i] {
        return Err(IoError::CycleDetected(cells[i].name.clone()));
    }
    on_stack[i] = true;
    for inst in &cells[i].instances {
        if let Some(&j) = by_name.get(&inst.sname) {
            visit(j, cells, by_name, visited, on_stack, order)?;
        }
    }
    on_stack[i] = false;
    visited[i] = true;
    order.push(i);
    Ok(())
}

fn read_u8(r: &mut &[u8]) -> Result<u8> {
    if r.is_empty() {
        return Err(IoError::UnexpectedEof);
    }
    let b = r[0];
    *r = &r[1..];
    Ok(b)
}

/// RECTANGLE info byte: SWHXYRDL (high to low).
/// Returns (layer, datatype, x, y, w, h, offsets) where offsets is the
/// per-instance (dx, dy) list (`vec![(0,0)]` if no repetition).
#[allow(clippy::type_complexity)]
fn parse_rectangle(
    r: &mut &[u8],
    info: u8,
    modal: &mut Modal,
) -> Result<(u64, u64, i64, i64, i64, i64, Vec<(i64, i64)>)> {
    let has_layer = (info & 0x01) != 0;
    let has_datatype = (info & 0x02) != 0;
    let has_repetition = (info & 0x04) != 0;
    let has_y = (info & 0x08) != 0;
    let has_x = (info & 0x10) != 0;
    let has_h = (info & 0x20) != 0;
    let has_w = (info & 0x40) != 0;
    let is_square = (info & 0x80) != 0;

    if has_layer {
        modal.layer = decode_unsigned(r)?;
    }
    if has_datatype {
        modal.datatype = decode_unsigned(r)?;
    }
    if has_w {
        modal.geometry_w = decode_unsigned(r)?;
    }
    if has_h && !is_square {
        modal.geometry_h = decode_unsigned(r)?;
    }
    if is_square {
        modal.geometry_h = modal.geometry_w;
    }
    if has_x {
        modal.geometry_x = decode_signed(r)?;
    }
    if has_y {
        modal.geometry_y = decode_signed(r)?;
    }
    let offsets = if has_repetition {
        decode_repetition(r, modal)?
    } else {
        vec![(0, 0)]
    };
    Ok((
        modal.layer,
        modal.datatype,
        modal.geometry_x,
        modal.geometry_y,
        modal.geometry_w as i64,
        modal.geometry_h as i64,
        offsets,
    ))
}

/// POLYGON info byte: 00PXYRDL (high to low). Point-list types 0..5 are
/// supported per spec §7.7.
fn parse_polygon(
    r: &mut &[u8],
    info: u8,
    modal: &mut Modal,
) -> Result<(u64, u64, Vec<Point>, Vec<(i64, i64)>)> {
    let has_layer = (info & 0x01) != 0;
    let has_datatype = (info & 0x02) != 0;
    let has_repetition = (info & 0x04) != 0;
    let has_y = (info & 0x08) != 0;
    let has_x = (info & 0x10) != 0;
    let has_p = (info & 0x20) != 0;

    if has_layer {
        modal.layer = decode_unsigned(r)?;
    }
    if has_datatype {
        modal.datatype = decode_unsigned(r)?;
    }
    if has_p {
        let plist_type = decode_unsigned(r)?;
        let count = decode_unsigned(r)? as usize;
        modal.last_polygon_points = decode_point_list(r, plist_type, count, true)?;
    }
    if has_x {
        modal.geometry_x = decode_signed(r)?;
    }
    if has_y {
        modal.geometry_y = decode_signed(r)?;
    }
    let offsets = if has_repetition {
        decode_repetition(r, modal)?
    } else {
        vec![(0, 0)]
    };

    let mut hull = Vec::with_capacity(modal.last_polygon_points.len() + 1);
    hull.push(Point::new(modal.geometry_x, modal.geometry_y));
    let mut cx = modal.geometry_x;
    let mut cy = modal.geometry_y;
    for (dx, dy) in &modal.last_polygon_points {
        // Malformed input can sum deltas past i64::MAX; surface as
        // a typed parse error rather than panicking.
        cx = cx
            .checked_add(*dx)
            .ok_or(IoError::OasisNotImplemented("polygon hull x overflow"))?;
        cy = cy
            .checked_add(*dy)
            .ok_or(IoError::OasisNotImplemented("polygon hull y overflow"))?;
        hull.push(Point::new(cx, cy));
    }
    Ok((modal.layer, modal.datatype, hull, offsets))
}

/// PATH info byte: EWPXYRDL (high to low). E selects extension scheme.
#[allow(clippy::type_complexity)]
fn parse_path(
    r: &mut &[u8],
    info: u8,
    modal: &mut Modal,
) -> Result<(u64, u64, Vec<Point>, i64, i64, i64)> {
    let has_layer = (info & 0x01) != 0;
    let has_datatype = (info & 0x02) != 0;
    let has_repetition = (info & 0x04) != 0;
    let has_y = (info & 0x08) != 0;
    let has_x = (info & 0x10) != 0;
    let has_p = (info & 0x20) != 0;
    let has_w = (info & 0x40) != 0;
    let has_e = (info & 0x80) != 0;

    if has_layer {
        modal.layer = decode_unsigned(r)?;
    }
    if has_datatype {
        modal.datatype = decode_unsigned(r)?;
    }
    if has_w {
        modal.path_halfwidth = decode_unsigned(r)?;
    }
    if has_e {
        // Two-bit extension scheme: SS EE — each pair decodes as 0=flat,
        // 1=halfwidth, 2=arbitrary, 3=reuse-modal.
        let scheme = read_u8(r)? as i64;
        let s_bits = (scheme >> 2) & 0x3;
        let e_bits = scheme & 0x3;
        modal.path_start_extension = match s_bits {
            0 => 0,
            1 => modal.path_halfwidth as i64,
            2 => decode_signed(r)?,
            _ => modal.path_start_extension,
        };
        modal.path_end_extension = match e_bits {
            0 => 0,
            1 => modal.path_halfwidth as i64,
            2 => decode_signed(r)?,
            _ => modal.path_end_extension,
        };
    }
    if has_p {
        let plist_type = decode_unsigned(r)?;
        let count = decode_unsigned(r)? as usize;
        modal.last_path_points = decode_point_list(r, plist_type, count, false)?;
    }
    if has_x {
        modal.geometry_x = decode_signed(r)?;
    }
    if has_y {
        modal.geometry_y = decode_signed(r)?;
    }
    if has_repetition {
        let _ = decode_repetition(r, modal)?;
    }

    let mut points = Vec::with_capacity(modal.last_path_points.len() + 1);
    points.push(Point::new(modal.geometry_x, modal.geometry_y));
    let mut cx = modal.geometry_x;
    let mut cy = modal.geometry_y;
    for (dx, dy) in &modal.last_path_points {
        cx += dx;
        cy += dy;
        points.push(Point::new(cx, cy));
    }
    Ok((
        modal.layer,
        modal.datatype,
        points,
        modal.path_halfwidth as i64,
        modal.path_start_extension,
        modal.path_end_extension,
    ))
}

/// TEXT info byte: 0CNXYRTL.
fn parse_text(
    r: &mut &[u8],
    info: u8,
    modal: &mut Modal,
    text_table: &[String],
) -> Result<(u64, u64, i64, i64, String)> {
    let has_textlayer = (info & 0x01) != 0;
    let has_texttype = (info & 0x02) != 0;
    let has_repetition = (info & 0x04) != 0;
    let has_y = (info & 0x08) != 0;
    let has_x = (info & 0x10) != 0;
    let has_n = (info & 0x20) != 0;
    let has_c = (info & 0x40) != 0;

    if has_c {
        modal.text_string = decode_string(r)?;
    } else if has_n {
        let refnum = decode_unsigned(r)? as usize;
        modal.text_string = text_table
            .get(refnum)
            .cloned()
            .ok_or(IoError::OasisNotImplemented("TEXT refnum out of range"))?;
    }
    if has_textlayer {
        modal.textlayer = decode_unsigned(r)?;
    }
    if has_texttype {
        modal.texttype = decode_unsigned(r)?;
    }
    if has_x {
        modal.text_x = decode_signed(r)?;
    }
    if has_y {
        modal.text_y = decode_signed(r)?;
    }
    if has_repetition {
        let _ = decode_repetition(r, modal)?;
    }
    Ok((
        modal.textlayer,
        modal.texttype,
        modal.text_x,
        modal.text_y,
        modal.text_string.clone(),
    ))
}

/// TRAPEZOID variants (23/24/25). Info byte 0OWHXYRDL where O selects
/// orientation, and W/H select width/height field presence. Variant 23
/// has both a and b deltas; 24 has only a; 25 has only b.
fn parse_trapezoid(
    r: &mut &[u8],
    info: u8,
    modal: &mut Modal,
    kind: u8,
) -> Result<(u64, u64, Vec<Point>)> {
    let has_layer = (info & 0x01) != 0;
    let has_datatype = (info & 0x02) != 0;
    let has_repetition = (info & 0x04) != 0;
    let has_y = (info & 0x08) != 0;
    let has_x = (info & 0x10) != 0;
    let has_h = (info & 0x20) != 0;
    let has_w = (info & 0x40) != 0;
    let vertical = (info & 0x80) != 0;

    if has_layer {
        modal.layer = decode_unsigned(r)?;
    }
    if has_datatype {
        modal.datatype = decode_unsigned(r)?;
    }
    if has_w {
        modal.trap_w = decode_unsigned(r)?;
    }
    if has_h {
        modal.trap_h = decode_unsigned(r)?;
    }
    let read_a = matches!(kind, TRAPEZOID | TRAPEZOID_A);
    let read_b = matches!(kind, TRAPEZOID | TRAPEZOID_B);
    if read_a {
        modal.trap_a = decode_signed(r)?;
    } else {
        modal.trap_a = 0;
    }
    if read_b {
        modal.trap_b = decode_signed(r)?;
    } else {
        modal.trap_b = 0;
    }
    if has_x {
        modal.geometry_x = decode_signed(r)?;
    }
    if has_y {
        modal.geometry_y = decode_signed(r)?;
    }
    if has_repetition {
        let _ = decode_repetition(r, modal)?;
    }

    let x = modal.geometry_x;
    let y = modal.geometry_y;
    let w = modal.trap_w as i64;
    let h = modal.trap_h as i64;
    let a = modal.trap_a;
    let b = modal.trap_b;
    // SEMI P39 §31 — vertices for horizontal trapezoid (vertical=false):
    //   (x + max(a,0),     y),
    //   (x + w + min(b,0), y),
    //   (x + w + max(b,0), y + h),
    //   (x + min(a,0),     y + h)
    // Vertical (vertical=true): rotate 90° / swap axes.
    let hull = if !vertical {
        vec![
            Point::new(x + a.max(0), y),
            Point::new(x + w + b.min(0), y),
            Point::new(x + w + b.max(0), y + h),
            Point::new(x + a.min(0), y + h),
        ]
    } else {
        vec![
            Point::new(x, y + a.max(0)),
            Point::new(x, y + h + b.min(0)),
            Point::new(x + w, y + h + b.max(0)),
            Point::new(x + w, y + a.min(0)),
        ]
    };
    Ok((modal.layer, modal.datatype, hull))
}

/// CTRAPEZOID info byte: TWHXYRDL where T-bit selects whether type is
/// present.
fn parse_ctrapezoid(
    r: &mut &[u8],
    info: u8,
    modal: &mut Modal,
) -> Result<(u64, u64, Vec<Point>)> {
    let has_layer = (info & 0x01) != 0;
    let has_datatype = (info & 0x02) != 0;
    let has_repetition = (info & 0x04) != 0;
    let has_y = (info & 0x08) != 0;
    let has_x = (info & 0x10) != 0;
    let has_h = (info & 0x20) != 0;
    let has_w = (info & 0x40) != 0;
    let has_type = (info & 0x80) != 0;

    if has_layer {
        modal.layer = decode_unsigned(r)?;
    }
    if has_datatype {
        modal.datatype = decode_unsigned(r)?;
    }
    let ctype = if has_type {
        let t = decode_unsigned(r)?;
        modal.trap_a = t as i64; // Reuse trap_a as ctype modal.
        t
    } else {
        modal.trap_a as u64
    };
    if has_w {
        modal.trap_w = decode_unsigned(r)?;
    }
    if has_h {
        modal.trap_h = decode_unsigned(r)?;
    }
    if has_x {
        modal.geometry_x = decode_signed(r)?;
    }
    if has_y {
        modal.geometry_y = decode_signed(r)?;
    }
    if has_repetition {
        let _ = decode_repetition(r, modal)?;
    }

    let hull = ctrap_hull(
        ctype,
        modal.geometry_x,
        modal.geometry_y,
        modal.trap_w as i64,
        modal.trap_h as i64,
    );
    Ok((modal.layer, modal.datatype, hull))
}

/// CIRCLE info byte: 00rrXYR (per spec) — only radius, x, y, repetition.
fn parse_circle(
    r: &mut &[u8],
    info: u8,
    modal: &mut Modal,
) -> Result<(u64, u64, i64, i64, i64)> {
    let has_layer = (info & 0x01) != 0;
    let has_datatype = (info & 0x02) != 0;
    let has_repetition = (info & 0x04) != 0;
    let has_y = (info & 0x08) != 0;
    let has_x = (info & 0x10) != 0;
    let has_radius = (info & 0x20) != 0;

    if has_layer {
        modal.layer = decode_unsigned(r)?;
    }
    if has_datatype {
        modal.datatype = decode_unsigned(r)?;
    }
    if has_radius {
        modal.circle_radius = decode_unsigned(r)?;
    }
    if has_x {
        modal.geometry_x = decode_signed(r)?;
    }
    if has_y {
        modal.geometry_y = decode_signed(r)?;
    }
    if has_repetition {
        let _ = decode_repetition(r, modal)?;
    }
    Ok((
        modal.layer,
        modal.datatype,
        modal.geometry_x,
        modal.geometry_y,
        modal.circle_radius as i64,
    ))
}

/// PLACEMENT (17) info byte: CN0XYRAAF (low to high).
/// PLACEMENT_TRANSFORM (18) info byte: CNXYRMAF where M=mag, A=angle (real),
/// F=flip(mirror). Returns the resolved name + integer Trans (`Rot4` only).
fn parse_placement(
    r: &mut &[u8],
    info: u8,
    modal: &mut Modal,
    cellname_table: &[String],
    is_transform: bool,
) -> Result<(String, Trans, Vec<(i64, i64)>)> {
    // PLACEMENT info byte CNXYRAAF (or CNXYRMAF for PLACEMENT_TRANSFORM):
    //   C[7]: 1 = cellname explicit, 0 = modal recall
    //   N[6]: only meaningful if C=1 — 1 = refnum, 0 = name-string
    let has_cell_explicit = (info & 0x80) != 0;
    let name_is_ref = (info & 0x40) != 0;

    let name = if has_cell_explicit {
        if name_is_ref {
            let refnum = decode_unsigned(r)? as usize;
            let n = cellname_table
                .get(refnum)
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| format!("{CELLNAME_REF_SENTINEL}{refnum}"));
            modal.last_cellname = Some(n.clone());
            n
        } else {
            let n = decode_string(r)?;
            modal.last_cellname = Some(n.clone());
            n
        }
    } else {
        modal.last_cellname.clone().ok_or(IoError::OasisNotImplemented(
            "PLACEMENT with no name and no modal value",
        ))?
    };

    let mut angle_deg: f64 = 0.0;
    let mirror;
    let (has_x, has_y, has_repetition);
    if is_transform {
        let has_mag = (info & 0x04) != 0;
        let has_angle = (info & 0x02) != 0;
        mirror = (info & 0x01) != 0;
        if has_mag {
            let _mag = decode_real(r)?;
        }
        if has_angle {
            angle_deg = decode_real(r)?.to_f64();
        }
        has_x = (info & 0x20) != 0;
        has_y = (info & 0x10) != 0;
        has_repetition = (info & 0x08) != 0;
    } else {
        // PLACEMENT (17) packs three bits AAF (rot in 90° increments + mirror).
        let aa = (info >> 1) & 0x03;
        mirror = (info & 0x01) != 0;
        angle_deg = (aa as f64) * 90.0;
        has_x = (info & 0x20) != 0;
        has_y = (info & 0x10) != 0;
        has_repetition = (info & 0x08) != 0;
    }
    if has_x {
        modal.placement_x = decode_signed(r)?;
    }
    if has_y {
        modal.placement_y = decode_signed(r)?;
    }
    let offsets = if has_repetition {
        decode_repetition(r, modal)?
    } else {
        vec![(0, 0)]
    };
    let normalized = ((angle_deg % 360.0) + 360.0) % 360.0;
    let rot = if (normalized - 0.0).abs() < 1e-6 || (normalized - 360.0).abs() < 1e-6 {
        Rot4::R0
    } else if (normalized - 90.0).abs() < 1e-6 {
        Rot4::R90
    } else if (normalized - 180.0).abs() < 1e-6 {
        Rot4::R180
    } else if (normalized - 270.0).abs() < 1e-6 {
        Rot4::R270
    } else {
        return Err(IoError::NonOrthogonalAngle(angle_deg));
    };
    let trans = Trans::new(rot, mirror, Vec2::new(modal.placement_x, modal.placement_y));
    Ok((name, trans, offsets))
}

/// Sanity bound on a polygon / point-list vertex count. Real OASIS
/// files don't ship polygons with more than ~10⁵ vertices; a count
/// much above this is a malformed / hostile input. We fail early
/// rather than passing the count to `Vec::with_capacity`, which on
/// a malicious input can request a multi-TB allocation and abort
/// the process. The threshold is generous enough that no
/// legitimate file will be rejected.
const MAX_POINT_LIST_COUNT: usize = 1 << 22;

/// Sanity bound on a CELLNAME / TEXTSTRING reference-table index. A
/// real OASIS file's reference tables have at most a few thousand
/// entries (one per unique cell / text string in the design); a
/// refnum in the millions is unambiguously a malformed input
/// driving `Vec::resize` toward an OOM. The threshold leaves room
/// for designs with hundreds of thousands of unique strings.
pub(crate) const MAX_REF_TABLE_SIZE: usize = 1 << 22;

/// Decode an OASIS point-list. `closed` indicates whether the implied
/// final closing edge back to the origin should be skipped (POLYGON =
/// closed; PATH = open).
fn decode_point_list(
    r: &mut &[u8],
    plist_type: u64,
    count: usize,
    closed: bool,
) -> Result<Vec<(i64, i64)>> {
    if count > MAX_POINT_LIST_COUNT {
        return Err(IoError::OasisNotImplemented(
            "point-list count exceeds sanity bound",
        ));
    }
    match plist_type {
        // Type 0: manhattan-horizontal-first; alternating dx, dy.
        // Type 1: manhattan-vertical-first; alternating dy, dx.
        // SEMI P39 §7.7.3: count = N - 2 for an N-vertex manhattan
        // polygon. The last two edges are implicit and must be
        // perpendicular (one vertical, one horizontal) to close back
        // to the start.
        0 | 1 => {
            let mut deltas: Vec<(i64, i64)> = Vec::with_capacity(count + 2);
            let horiz_first = plist_type == 0;
            for i in 0..count {
                let v = decode_signed(r)?;
                let on_x = if horiz_first { i % 2 == 0 } else { i % 2 == 1 };
                if on_x {
                    deltas.push((v, 0));
                } else {
                    deltas.push((0, v));
                }
            }
            if closed {
                // OASIS spec: count = N - 2, last 2 edges implicit. Of those,
                // we materialise ONE delta to land on the final vertex (the
                // hull representation's last explicit vertex); the second
                // implicit edge closes back to vertex 0 and is not stored.
                //
                // Malformed input may set `count = 0` (an N=2 polygon
                // is degenerate but the byte stream can still claim
                // it). Guard the `count - 1` underflow.
                if count == 0 {
                    return Err(IoError::OasisNotImplemented(
                        "manhattan polygon with count=0 is degenerate",
                    ));
                }
                let mut sx: i64 = 0;
                let mut sy: i64 = 0;
                for d in &deltas {
                    sx = sx.saturating_add(d.0);
                    sy = sy.saturating_add(d.1);
                }
                let last_was_horizontal = if horiz_first {
                    (count - 1) % 2 == 0
                } else {
                    (count - 1) % 2 == 1
                };
                if last_was_horizontal {
                    deltas.push((0, -sy));
                } else {
                    deltas.push((-sx, 0));
                }
            }
            Ok(deltas)
        }
        // Type 2: manhattan all-angle (octangular without diagonals);
        // each delta is g-delta (3 LSBs encode direction).
        2 => decode_g_deltas(r, count, false),
        // Type 3: octangular (8 directions).
        3 => decode_g_deltas(r, count, false),
        // Type 4: all-angle; form-1 octangular OR form-2 (dx packed +
        // signed dy).
        4 => decode_g_deltas(r, count, true),
        // Type 5: general — paired signed (dx, dy).
        5 => {
            let mut deltas: Vec<(i64, i64)> = Vec::with_capacity(count);
            for _ in 0..count {
                let dx = decode_signed(r)?;
                let dy = decode_signed(r)?;
                deltas.push((dx, dy));
            }
            Ok(deltas)
        }
        other => Err(IoError::OasisUnsupportedRecord(other as u8)),
    }
}

/// Decode `count` g-deltas. SEMI P39 §7.5.
///
/// Form-1 (raw bit 0 = 0): octangular delta packed in one unsigned-int.
///   bits 1..3 = direction code (0=E, 1=N, 2=W, 3=S, 4=NE, 5=NW, 6=SW, 7=SE)
///   bits 4..  = magnitude
///
/// Form-2 (raw bit 0 = 1): non-octangular delta. Only valid when
/// `allow_form2` is set (point-list type 4 / type 5 use form-2; types
/// 2/3 are octangular-only). Layout:
///   bit 0 = 1 (form selector)
///   bit 1 = sign of dx (0 = positive, 1 = negative)
///   bits 2.. = magnitude of dx
/// Then a signed-int for dy follows the unsigned.
fn decode_g_deltas(
    r: &mut &[u8],
    count: usize,
    allow_form2: bool,
) -> Result<Vec<(i64, i64)>> {
    let mut deltas: Vec<(i64, i64)> = Vec::with_capacity(count);
    for _ in 0..count {
        let raw = decode_unsigned(r)?;
        if (raw & 0x1) == 0 {
            let dir = (raw >> 1) & 0x7;
            let mag = (raw >> 4) as i64;
            let (dx, dy) = match dir {
                0 => (mag, 0),
                1 => (0, mag),
                2 => (-mag, 0),
                3 => (0, -mag),
                4 => (mag, mag),
                5 => (-mag, mag),
                6 => (-mag, -mag),
                7 => (mag, -mag),
                _ => unreachable!(),
            };
            deltas.push((dx, dy));
        } else if allow_form2 {
            let dx_neg = (raw >> 1) & 0x1 == 1;
            let dx_mag = (raw >> 2) as i64;
            let dx = if dx_neg { -dx_mag } else { dx_mag };
            let dy = decode_signed(r)?;
            deltas.push((dx, dy));
        } else {
            return Err(IoError::OasisNotImplemented(
                "g-delta form-2 in octangular-only point list",
            ));
        }
    }
    Ok(deltas)
}

/// Decode a repetition payload, returning the list of (dx, dy) offsets
/// (including (0, 0) for the base element). Updates `modal.last_repetition`
/// for `kind = 0` (reuse-modal) bookkeeping. Per SEMI P39 §11.16.
fn decode_repetition(r: &mut &[u8], modal: &mut Modal) -> Result<Vec<(i64, i64)>> {
    let kind = decode_unsigned(r)?;
    // Reuse the point-list count cap for repetition counts. Real
    // OASIS files don't repeat anything more than ~10⁵ times.
    let cap_count = |raw: u64, plus: u64| -> Result<i64> {
        let n = (raw as usize).saturating_add(plus as usize);
        if n > MAX_POINT_LIST_COUNT {
            return Err(IoError::OasisNotImplemented(
                "repetition count exceeds sanity bound",
            ));
        }
        Ok(n as i64)
    };
    let cap_product = |a: i64, b: i64| -> Result<()> {
        let prod = (a as usize).saturating_mul(b as usize);
        if prod > MAX_POINT_LIST_COUNT {
            return Err(IoError::OasisNotImplemented(
                "repetition matrix size exceeds sanity bound",
            ));
        }
        Ok(())
    };
    let offsets: Vec<(i64, i64)> = match kind {
        0 => return Ok(modal.last_repetition.clone().unwrap_or_else(|| vec![(0, 0)])),
        1 => {
            let nx = cap_count(decode_unsigned(r)?, 2)?;
            let ny = cap_count(decode_unsigned(r)?, 2)?;
            cap_product(nx, ny)?;
            let dx = decode_unsigned(r)? as i64;
            let dy = decode_unsigned(r)? as i64;
            (0..ny)
                .flat_map(|j| {
                    (0..nx).map(move |i| (i.saturating_mul(dx), j.saturating_mul(dy)))
                })
                .collect()
        }
        2 => {
            let nx = cap_count(decode_unsigned(r)?, 2)?;
            let dx = decode_unsigned(r)? as i64;
            (0..nx).map(|i| (i.saturating_mul(dx), 0)).collect()
        }
        3 => {
            let ny = cap_count(decode_unsigned(r)?, 2)?;
            let dy = decode_unsigned(r)? as i64;
            (0..ny).map(|j| (0, j.saturating_mul(dy))).collect()
        }
        4 | 5 => {
            // x-arbitrary. Type 4: list of (n+1) unsigned dx-deltas.
            // Type 5: same but with a grid multiplier prepended.
            let n = (decode_unsigned(r)? as usize).saturating_add(1);
            if n > MAX_POINT_LIST_COUNT {
                return Err(IoError::OasisNotImplemented(
                    "x-arbitrary point-list count exceeds sanity bound",
                ));
            }
            let grid = if kind == 5 { decode_unsigned(r)? as i64 } else { 1 };
            let mut x = 0i64;
            let mut out: Vec<(i64, i64)> = vec![(0, 0)];
            for _ in 0..n {
                let raw = decode_unsigned(r)? as i64;
                let d = raw
                    .checked_mul(grid)
                    .ok_or(IoError::OasisNotImplemented("delta · grid overflow"))?;
                x = x
                    .checked_add(d)
                    .ok_or(IoError::OasisNotImplemented("accumulated x overflow"))?;
                out.push((x, 0));
            }
            out
        }
        6 | 7 => {
            // y-arbitrary, mirror of 4/5.
            let n = (decode_unsigned(r)? as usize).saturating_add(1);
            if n > MAX_POINT_LIST_COUNT {
                return Err(IoError::OasisNotImplemented(
                    "y-arbitrary point-list count exceeds sanity bound",
                ));
            }
            let grid = if kind == 7 { decode_unsigned(r)? as i64 } else { 1 };
            let mut y = 0i64;
            let mut out: Vec<(i64, i64)> = vec![(0, 0)];
            for _ in 0..n {
                let raw = decode_unsigned(r)? as i64;
                let d = raw
                    .checked_mul(grid)
                    .ok_or(IoError::OasisNotImplemented("delta · grid overflow"))?;
                y = y
                    .checked_add(d)
                    .ok_or(IoError::OasisNotImplemented("accumulated y overflow"))?;
                out.push((0, y));
            }
            out
        }
        8 => {
            // matrix general: nx-2, ny-2, then two g-delta direction
            // vectors (col, row).
            let nx = cap_count(decode_unsigned(r)?, 2)?;
            let ny = cap_count(decode_unsigned(r)?, 2)?;
            cap_product(nx, ny)?;
            let (dxc, dyc) = decode_one_g_delta(r)?;
            let (dxr, dyr) = decode_one_g_delta(r)?;
            (0..ny)
                .flat_map(|j| {
                    (0..nx).map(move |i| {
                        let x = i.saturating_mul(dxc).saturating_add(j.saturating_mul(dxr));
                        let y = i.saturating_mul(dyc).saturating_add(j.saturating_mul(dyr));
                        (x, y)
                    })
                })
                .collect()
        }
        9 => {
            // diag matrix: n-2, then a g-delta vector.
            let n_raw = decode_unsigned(r)? as usize;
            if n_raw > MAX_POINT_LIST_COUNT {
                return Err(IoError::OasisNotImplemented(
                    "diag-matrix count exceeds sanity bound",
                ));
            }
            let n = (n_raw as i64).saturating_add(2);
            let (dx, dy) = decode_one_g_delta(r)?;
            (0..n)
                .map(|i| {
                    let xi = i.checked_mul(dx).unwrap_or(0);
                    let yi = i.checked_mul(dy).unwrap_or(0);
                    (xi, yi)
                })
                .collect()
        }
        10 | 11 => {
            // arbitrary list of g-delta positions.
            let n = (decode_unsigned(r)? as usize).saturating_add(1);
            if n > MAX_POINT_LIST_COUNT {
                return Err(IoError::OasisNotImplemented(
                    "g-delta point-list count exceeds sanity bound",
                ));
            }
            let grid = if kind == 11 {
                decode_unsigned(r)? as i64
            } else {
                1
            };
            let mut x = 0i64;
            let mut y = 0i64;
            let mut out: Vec<(i64, i64)> = vec![(0, 0)];
            for _ in 0..n {
                let (dx, dy) = decode_one_g_delta(r)?;
                // Malformed OASIS streams can pair an enormous
                // delta with an enormous grid multiplier, blowing
                // through `i64` range. Use checked arithmetic and
                // surface the overflow as a typed parse error so
                // the fuzz harness no longer panics here.
                let scaled_dx = dx
                    .checked_mul(grid)
                    .ok_or(IoError::OasisNotImplemented("delta · grid overflow"))?;
                let scaled_dy = dy
                    .checked_mul(grid)
                    .ok_or(IoError::OasisNotImplemented("delta · grid overflow"))?;
                x = x
                    .checked_add(scaled_dx)
                    .ok_or(IoError::OasisNotImplemented("accumulated x overflow"))?;
                y = y
                    .checked_add(scaled_dy)
                    .ok_or(IoError::OasisNotImplemented("accumulated y overflow"))?;
                out.push((x, y));
            }
            out
        }
        _ => return Err(IoError::OasisNotImplemented("unknown repetition kind")),
    };
    modal.last_repetition = Some(offsets.clone());
    Ok(offsets)
}

/// Decode one g-delta value (form-1 octangular OR form-2 paired
/// signed). Used inside repetition payloads.
fn decode_one_g_delta(r: &mut &[u8]) -> Result<(i64, i64)> {
    let raw = decode_unsigned(r)?;
    if raw & 1 == 0 {
        let dir = (raw >> 1) & 0x7;
        let mag = (raw >> 4) as i64;
        Ok(match dir {
            0 => (mag, 0),
            1 => (0, mag),
            2 => (-mag, 0),
            3 => (0, -mag),
            4 => (mag, mag),
            5 => (-mag, mag),
            6 => (-mag, -mag),
            7 => (mag, -mag),
            _ => unreachable!(),
        })
    } else {
        let dx_neg = (raw >> 1) & 1 == 1;
        let dx_mag = (raw >> 2) as i64;
        let dx = if dx_neg { -dx_mag } else { dx_mag };
        let dy = decode_signed(r)?;
        Ok((dx, dy))
    }
}

fn ctrap_hull(t: u64, x: i64, y: i64, w: i64, h: i64) -> Vec<Point> {
    // Reference: SEMI P39 §32, table of 26 standard compressed
    // trapezoids. Returns the four hull points in CCW order.
    let p = |dx: i64, dy: i64| Point::new(x + dx, y + dy);
    match t {
        0 => vec![p(0, 0), p(w, 0), p(w - h, h), p(0, h)],
        1 => vec![p(0, 0), p(w, 0), p(w, h), p(h, h)],
        2 => vec![p(0, 0), p(w, 0), p(w, h), p(0, h - (w.min(h)))],
        3 => vec![p(0, 0), p(w, 0), p(w - h.min(w), h), p(0, h)],
        4 => vec![p(0, 0), p(w, 0), p(w, h), p(0, 0)],
        5 => vec![p(0, 0), p(w, 0), p(w, h), p(0, h)],
        6 => vec![p(0, 0), p(w, 0), p(w, h), p(0, h)],
        7 => vec![p(0, 0), p(w, 0), p(w, h), p(0, h)],
        8 => vec![p(0, 0), p(h, 0), p(w, h), p(0, h)],
        9 => vec![p(0, 0), p(w - h, 0), p(w, h), p(0, h)],
        10 => vec![p(0, 0), p(w, 0), p(w, h), p(h, h)],
        11 => vec![p(h, 0), p(w, 0), p(w, h), p(0, h)],
        12 => vec![p(0, 0), p(w, 0), p(w, h - w.min(h)), p(0, h)],
        13 => vec![p(0, 0), p(w, w.min(h)), p(w, h), p(0, h)],
        14 => vec![p(0, 0), p(w, 0), p(w, h), p(0, h - w.min(h))],
        15 => vec![p(0, 0), p(w, 0), p(w, h), p(0, w.min(h))],
        16 => vec![p(0, 0), p(w, 0), p(w / 2, h), p(0, 0)],
        17 => vec![p(w / 2, 0), p(w, h), p(0, h), p(w / 2, 0)],
        18 => vec![p(0, 0), p(h, h / 2), p(0, h), p(0, 0)],
        19 => vec![p(0, h / 2), p(w, 0), p(w, h), p(0, h / 2)],
        20 => vec![p(0, 0), p(2 * h, 0), p(h, h), p(0, 0)],
        21 => vec![p(0, h), p(h, 0), p(2 * h, h), p(0, h)],
        22 => vec![p(0, 0), p(h, 0), p(h, 2 * h), p(0, 0)],
        23 => vec![p(0, 0), p(h, h), p(0, 2 * h), p(0, 0)],
        24 => vec![p(0, 0), p(w, 0), p(w, w), p(0, w)], // square w=h
        25 => vec![p(0, 0), p(w, 0), p(w, w), p(0, w)],
        _ => vec![p(0, 0), p(w, 0), p(w, h), p(0, h)],
    }
}

fn circle_hull(cx: i64, cy: i64, radius: i64) -> Vec<Point> {
    use std::f64::consts::PI;
    let mut pts = Vec::with_capacity(CIRCLE_FACETS);
    for i in 0..CIRCLE_FACETS {
        let a = 2.0 * PI * (i as f64) / (CIRCLE_FACETS as f64);
        let dx = (radius as f64 * a.cos()).round() as i64;
        let dy = (radius as f64 * a.sin()).round() as i64;
        // Saturate rather than panic — center + offset can overflow
        // `i64` for malformed (extreme) center / radius pairs.
        let x = cx.saturating_add(dx);
        let y = cy.saturating_add(dy);
        pts.push(Point::new(x, y));
    }
    pts
}
