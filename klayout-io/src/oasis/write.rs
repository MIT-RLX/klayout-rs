//! OASIS writer.
//!
//! v3 emits a valid OASIS file containing:
//! * `START` with version + DBU + offset-flag.
//! * Per cell: `CELL` (with name) + shape records + `PLACEMENT` /
//!   `PLACEMENT_TRANSFORM` records for child instances.
//! * Shape records: `RECTANGLE` (info SWHXYRDL with all fields explicit),
//!   `POLYGON` (info 00PXYRDL with point-list type 5), `PATH` (info
//!   EWPXYRDL — emits halfwidth, extension scheme, and general
//!   point-list), `TEXT` (info 0CNXYRTL with explicit text string).
//! * `END` with zero offsets and no validation.
//!
//! All fields are emitted explicitly (no modal-variable compression on
//! write) so the file is self-contained and easy for the reader to
//! handle. CBLOCK compression is deferred. Rotated/mirrored placements
//! use `PLACEMENT_TRANSFORM`.

use super::codec::*;
use super::records::*;
use crate::error::Result;
use klayout_core::{Cell, CellId, Library, Point as CorePoint, Rot4, Shape};
use std::collections::HashMap;
use std::path::Path;

pub fn write_oasis_path(lib: &Library, path: impl AsRef<Path>) -> Result<()> {
    let buf = write_oasis_bytes(lib)?;
    std::fs::write(path.as_ref(), buf)?;
    Ok(())
}

/// Per-cell body size threshold above which the writer emits the cell
/// as a CBLOCK. Smaller cells are emitted uncompressed — the CBLOCK
/// header overhead (~3-5 bytes) wins back only on bigger payloads.
const CBLOCK_THRESHOLD: usize = 256;

pub fn write_oasis_bytes(lib: &Library) -> Result<Vec<u8>> {
    write_oasis_bytes_with(lib, CBLOCK_THRESHOLD)
}

/// Write OASIS bytes with an explicit CBLOCK threshold. `threshold` of
/// `usize::MAX` disables compression entirely (handy for tests that
/// want byte-stable output).
pub fn write_oasis_bytes_with(lib: &Library, cblock_threshold: usize) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    buf.extend_from_slice(MAGIC);

    buf.push(START);
    encode_string("1", &mut buf);
    encode_real(Real::PositiveWhole(lib.dbu() as u64), &mut buf);
    // offset_flag = 0 → the 12 table-offsets are emitted at the END
    // record (which we do, see below). Setting this to 1 would
    // instead require the offsets to follow this byte right inside
    // the START record. The reader desynchronizes if these don't
    // agree, so this constant must match the END layout.
    encode_unsigned(0, &mut buf);

    let cells = lib.all_cells();
    let id_to_idx: HashMap<CellId, usize> = cells
        .iter()
        .enumerate()
        .map(|(i, (id, _))| (*id, i))
        .collect();
    let order = topo_order(&cells, &id_to_idx);
    let id_to_name: HashMap<CellId, String> = cells
        .iter()
        .map(|(id, c)| (*id, c.name().as_str().to_string()))
        .collect();

    for idx in order {
        let (_id, cell) = &cells[idx];
        let mut cell_buf = Vec::new();
        write_cell(&mut cell_buf, cell, lib, &id_to_name);
        if cell_buf.len() >= cblock_threshold {
            emit_cblock(&mut buf, &cell_buf);
        } else {
            buf.extend_from_slice(&cell_buf);
        }
    }

    buf.push(END);
    for _ in 0..12 {
        encode_unsigned(0, &mut buf);
    }
    encode_unsigned(0, &mut buf);
    Ok(buf)
}

/// Compress `payload` with raw deflate and emit a CBLOCK record. The
/// reader's CBLOCK handler will splice the inflated bytes back into
/// the stream, so any sequence of records is valid here.
fn emit_cblock(buf: &mut Vec<u8>, payload: &[u8]) {
    use flate2::write::DeflateEncoder;
    use flate2::Compression;
    use std::io::Write;
    let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
    // Both calls write into a Vec<u8>, which never returns an io::Error;
    // the Result is part of the Write trait but is structurally Ok here.
    enc.write_all(payload)
        .expect("DeflateEncoder over Vec<u8> cannot fail");
    let compressed = enc
        .finish()
        .expect("DeflateEncoder::finish over Vec<u8> cannot fail");
    buf.push(CBLOCK);
    encode_unsigned(0, buf); // comp-type 0 = deflate
    encode_unsigned(payload.len() as u64, buf);
    encode_unsigned(compressed.len() as u64, buf);
    buf.extend_from_slice(&compressed);
}

fn topo_order(
    cells: &[(CellId, std::sync::Arc<Cell>)],
    id_to_idx: &HashMap<CellId, usize>,
) -> Vec<usize> {
    let mut visited = vec![false; cells.len()];
    let mut order = Vec::with_capacity(cells.len());
    fn visit(
        i: usize,
        cells: &[(CellId, std::sync::Arc<Cell>)],
        id_to_idx: &HashMap<CellId, usize>,
        visited: &mut [bool],
        order: &mut Vec<usize>,
    ) {
        if visited[i] {
            return;
        }
        visited[i] = true;
        for inst in cells[i].1.instances() {
            if let Some(&j) = id_to_idx.get(&inst.cell) {
                visit(j, cells, id_to_idx, visited, order);
            }
        }
        order.push(i);
    }
    for i in 0..cells.len() {
        visit(i, cells, id_to_idx, &mut visited, &mut order);
    }
    order
}

fn write_cell(
    buf: &mut Vec<u8>,
    cell: &Cell,
    lib: &Library,
    id_to_name: &HashMap<CellId, String>,
) {
    buf.push(CELL);
    encode_string(cell.name().as_str(), buf);

    for layer_idx in cell.layers() {
        let info = lib.layer_info(layer_idx);
        for shape in cell.shapes_on(layer_idx) {
            match shape {
                Shape::Box(r) => write_rectangle(
                    buf,
                    info.layer,
                    info.datatype,
                    r.bbox.min.x,
                    r.bbox.min.y,
                    r.bbox.width(),
                    r.bbox.height(),
                ),
                Shape::Polygon(p) if !p.hull.is_empty() => {
                    write_polygon(buf, info.layer, info.datatype, &p.hull)
                }
                Shape::Path(p) if p.points.len() >= 2 => write_path(
                    buf,
                    info.layer,
                    info.datatype,
                    &p.points,
                    p.width / 2,
                    p.begin_ext,
                    p.end_ext,
                ),
                Shape::Text(t) => write_text(
                    buf,
                    info.layer,
                    info.datatype,
                    &t.string,
                    t.anchor.x,
                    t.anchor.y,
                ),
                _ => {}
            }
        }
    }

    for inst in cell.instances() {
        let name = id_to_name.get(&inst.cell);
        if let Some(name) = name {
            if inst.trans.rot != Rot4::R0 || inst.trans.mirror {
                write_placement_transform(
                    buf,
                    name,
                    inst.trans.rot,
                    inst.trans.mirror,
                    inst.trans.disp.x,
                    inst.trans.disp.y,
                );
            } else {
                write_placement(buf, name, inst.trans.disp.x, inst.trans.disp.y);
            }
        }
    }
}

fn write_rectangle(
    buf: &mut Vec<u8>,
    layer: u16,
    datatype: u16,
    x: i64,
    y: i64,
    w: i64,
    h: i64,
) {
    buf.push(RECTANGLE);
    let info: u8 = 0b0111_1011;
    buf.push(info);
    encode_unsigned(layer as u64, buf);
    encode_unsigned(datatype as u64, buf);
    encode_unsigned(w as u64, buf);
    encode_unsigned(h as u64, buf);
    encode_signed(x, buf);
    encode_signed(y, buf);
}

fn write_polygon(buf: &mut Vec<u8>, layer: u16, datatype: u16, hull: &[CorePoint]) {
    buf.push(POLYGON);
    let info: u8 = 0b0011_1011;
    buf.push(info);
    encode_unsigned(layer as u64, buf);
    encode_unsigned(datatype as u64, buf);
    encode_unsigned(5, buf);
    encode_unsigned((hull.len() - 1) as u64, buf);
    for i in 1..hull.len() {
        let dx = hull[i].x - hull[i - 1].x;
        let dy = hull[i].y - hull[i - 1].y;
        encode_signed(dx, buf);
        encode_signed(dy, buf);
    }
    encode_signed(hull[0].x, buf);
    encode_signed(hull[0].y, buf);
}

fn write_path(
    buf: &mut Vec<u8>,
    layer: u16,
    datatype: u16,
    pts: &[CorePoint],
    halfwidth: i64,
    start_ext: i64,
    end_ext: i64,
) {
    buf.push(PATH);
    // Info EWPXYRDL: E (extension), W (halfwidth), P (point-list), X, Y,
    // no R, D (datatype), L (layer) — 0b1111_1011.
    let info: u8 = 0b1111_1011;
    buf.push(info);
    encode_unsigned(layer as u64, buf);
    encode_unsigned(datatype as u64, buf);
    encode_unsigned(halfwidth.max(0) as u64, buf);
    // Extension scheme: SS EE — both 2 = arbitrary signed payload follows.
    let scheme: u8 = (2 << 2) | 2;
    buf.push(scheme);
    encode_signed(start_ext, buf);
    encode_signed(end_ext, buf);
    // Point-list type 5 (general unconstrained), count = N - 1.
    encode_unsigned(5, buf);
    encode_unsigned((pts.len() - 1) as u64, buf);
    for i in 1..pts.len() {
        let dx = pts[i].x - pts[i - 1].x;
        let dy = pts[i].y - pts[i - 1].y;
        encode_signed(dx, buf);
        encode_signed(dy, buf);
    }
    encode_signed(pts[0].x, buf);
    encode_signed(pts[0].y, buf);
}

fn write_text(buf: &mut Vec<u8>, layer: u16, datatype: u16, s: &str, x: i64, y: i64) {
    buf.push(TEXT);
    // Info 0CNXYRTL — C (text string explicit), no N, has X, Y, no R,
    // T (texttype), L (textlayer) → 0b0101_1011.
    let info: u8 = 0b0101_1011;
    buf.push(info);
    encode_string(s, buf);
    encode_unsigned(layer as u64, buf);
    encode_unsigned(datatype as u64, buf);
    encode_signed(x, buf);
    encode_signed(y, buf);
}

fn write_placement(buf: &mut Vec<u8>, sname: &str, x: i64, y: i64) {
    buf.push(PLACEMENT);
    let info: u8 = 0b1011_0000;
    buf.push(info);
    encode_string(sname, buf);
    encode_signed(x, buf);
    encode_signed(y, buf);
}

fn write_placement_transform(
    buf: &mut Vec<u8>,
    sname: &str,
    rot: Rot4,
    mirror: bool,
    x: i64,
    y: i64,
) {
    buf.push(PLACEMENT_TRANSFORM);
    // Info CNXYRMAF: C=1, N=0, X=1, Y=1, R=0, M=0 (no mag), A=1 (angle real),
    // F=mirror flag.
    let mut info: u8 = 0b1011_0010;
    if mirror {
        info |= 0x01;
    }
    buf.push(info);
    encode_string(sname, buf);
    let angle = match rot {
        Rot4::R0 => 0.0_f64,
        Rot4::R90 => 90.0_f64,
        Rot4::R180 => 180.0_f64,
        Rot4::R270 => 270.0_f64,
    };
    encode_real(Real::Float64(angle), buf);
    encode_signed(x, buf);
    encode_signed(y, buf);
}
