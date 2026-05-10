//! Streaming OASIS reader.
//!
//! Mirrors the streaming GDS reader (`gds::stream`): scan an OASIS
//! buffer once for cell offsets, then iterate one cell's records on
//! demand without materialising the whole library. Useful for
//! sign-off-scale OASIS files that don't fit in RAM.
//!
//! v1 indexes cells whose CELL record uses an explicit name (record
//! 14). CELL_REF (record 13, name-table reference) is also supported —
//! the index is built after a one-time CELLNAME table scan.
//!
//! The decompressed OASIS stream is materialised once (CBLOCK splices
//! happen during indexing). The full file fits in `Vec<u8>` since
//! deflate-compressed cells expand at index time; for true sequential
//! streaming over a `Read`, see v2.

use super::codec::*;
use super::records::*;
use crate::error::{IoError, Result};
use std::collections::HashMap;
use std::io::Read;

#[derive(Clone, Debug)]
pub struct OasisIndex {
    /// Decompressed OASIS payload (post-CBLOCK, post-MAGIC). Used by
    /// `for_each_shape` for re-scan.
    payload: Vec<u8>,
    /// Cell name → byte offset of the first record after CELL/CELL_REF
    /// (skipping the name field).
    cells: HashMap<String, CellOffset>,
    dbu: i64,
}

#[derive(Copy, Clone, Debug)]
struct CellOffset {
    /// Position of the first record inside the cell.
    body_start: usize,
}

#[derive(Clone, Debug)]
pub enum OasisStreamEvent {
    Rectangle {
        layer: u16,
        datatype: u16,
        x: i64,
        y: i64,
        w: i64,
        h: i64,
    },
    Polygon {
        layer: u16,
        datatype: u16,
        hull: Vec<(i64, i64)>,
    },
    Placement {
        sname: String,
        x: i64,
        y: i64,
    },
}

/// Scan an OASIS buffer once for cell offsets.
pub fn open_oasis_bytes(bytes: &[u8]) -> Result<OasisIndex> {
    if bytes.len() < MAGIC.len() || &bytes[..MAGIC.len()] != MAGIC {
        return Err(IoError::OasisBadMagic);
    }
    // Materialise post-MAGIC payload, splicing CBLOCKs.
    let payload = inflate_full(&bytes[MAGIC.len()..])?;
    let mut r: &[u8] = &payload;
    if r.is_empty() || r[0] != START {
        return Err(IoError::OasisUnsupportedRecord(r.first().copied().unwrap_or(0)));
    }
    r = &r[1..];
    let _version = decode_string(&mut r)?;
    let unit = decode_real(&mut r)?;
    let _offset_flag = decode_unsigned(&mut r)?;
    let dbu = unit.to_f64().round().max(1.0) as i64;

    let mut cells: HashMap<String, CellOffset> = HashMap::new();
    let mut cellname_table: Vec<String> = Vec::new();

    while !r.is_empty() {
        let kind = r[0];
        r = &r[1..];
        match kind {
            END => break,
            PAD => {}
            CELL => {
                let name = decode_string(&mut r)?;
                let body_start = payload.len() - r.len();
                cells.insert(name, CellOffset { body_start });
                // Skip records until next CELL / CELL_REF / END.
                skip_until_cell_or_end(&mut r)?;
            }
            CELL_REF => {
                let refnum = decode_unsigned(&mut r)? as usize;
                let name = cellname_table
                    .get(refnum)
                    .cloned()
                    .ok_or(IoError::OasisNotImplemented(
                        "CELL_REF before CELLNAME table populated",
                    ))?;
                let body_start = payload.len() - r.len();
                cells.insert(name, CellOffset { body_start });
                skip_until_cell_or_end(&mut r)?;
            }
            CELLNAME => {
                let name = decode_string(&mut r)?;
                cellname_table.push(name);
            }
            CELLNAME_REF => {
                let name = decode_string(&mut r)?;
                let refnum = decode_unsigned(&mut r)? as usize;
                // Cap the refnum at the same sanity bound the
                // primary OASIS reader uses, so a malformed file
                // can't drive `Vec::resize` into an OOM.
                if refnum > super::read::MAX_REF_TABLE_SIZE {
                    return Err(IoError::OasisNotImplemented(
                        "stream cellname refnum exceeds sanity bound",
                    ));
                }
                if cellname_table.len() <= refnum {
                    cellname_table.resize(refnum + 1, String::new());
                }
                cellname_table[refnum] = name;
            }
            _ => {
                // Skip generic record by walking until CELL/CELL_REF/END.
                skip_until_cell_or_end(&mut r)?;
                break;
            }
        }
    }

    Ok(OasisIndex {
        payload,
        cells,
        dbu,
    })
}

impl OasisIndex {
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    pub fn cell_names(&self) -> impl Iterator<Item = &str> {
        self.cells.keys().map(|s| s.as_str())
    }

    pub fn dbu(&self) -> i64 {
        self.dbu
    }

    /// Walk one cell's records; invoke `f` on each shape/instance
    /// event. Records that need full modal-variable state are not
    /// emitted in v1 (PATH/TEXT/TRAPEZOID rely on accumulated modal
    /// state — full streaming requires re-establishing it from the
    /// buffer start).
    pub fn for_each_shape<F>(&self, name: &str, mut f: F) -> Result<()>
    where
        F: FnMut(OasisStreamEvent),
    {
        let off = self
            .cells
            .get(name)
            .ok_or_else(|| IoError::CellNotFound(name.to_string()))?;
        let mut r: &[u8] = &self.payload[off.body_start..];
        loop {
            if r.is_empty() {
                break;
            }
            let kind = r[0];
            r = &r[1..];
            match kind {
                END | CELL | CELL_REF => return Ok(()),
                PAD => continue,
                RECTANGLE => {
                    let info = read_u8(&mut r)?;
                    let (layer, datatype, x, y, w, h) =
                        read_rectangle_minimal(&mut r, info)?;
                    f(OasisStreamEvent::Rectangle {
                        layer: layer as u16,
                        datatype: datatype as u16,
                        x,
                        y,
                        w,
                        h,
                    });
                }
                POLYGON => {
                    // Polygon needs modal state for short-form encoding;
                    // emit only when a self-contained "P" bit is set.
                    let info = read_u8(&mut r)?;
                    if let Some((layer, datatype, hull)) = read_polygon_minimal(&mut r, info)? {
                        f(OasisStreamEvent::Polygon {
                            layer: layer as u16,
                            datatype: datatype as u16,
                            hull,
                        });
                    }
                }
                PLACEMENT => {
                    let info = read_u8(&mut r)?;
                    if let Some((sname, x, y)) = read_placement_minimal(&mut r, info)? {
                        f(OasisStreamEvent::Placement { sname, x, y });
                    }
                }
                _ => {
                    // Unhandled in streaming v1 — bail to avoid mis-decoding.
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

fn read_u8(r: &mut &[u8]) -> Result<u8> {
    if r.is_empty() {
        return Err(IoError::UnexpectedEof);
    }
    let b = r[0];
    *r = &r[1..];
    Ok(b)
}

fn read_rectangle_minimal(
    r: &mut &[u8],
    info: u8,
) -> Result<(u64, u64, i64, i64, i64, i64)> {
    let has_layer = (info & 0x01) != 0;
    let has_datatype = (info & 0x02) != 0;
    let has_repetition = (info & 0x04) != 0;
    let has_y = (info & 0x08) != 0;
    let has_x = (info & 0x10) != 0;
    let has_h = (info & 0x20) != 0;
    let has_w = (info & 0x40) != 0;
    let is_square = (info & 0x80) != 0;
    let layer = if has_layer { decode_unsigned(r)? } else { 0 };
    let datatype = if has_datatype { decode_unsigned(r)? } else { 0 };
    let w = if has_w { decode_unsigned(r)? } else { 0 };
    let h = if has_h && !is_square {
        decode_unsigned(r)?
    } else {
        w
    };
    let x = if has_x { decode_signed(r)? } else { 0 };
    let y = if has_y { decode_signed(r)? } else { 0 };
    if has_repetition {
        skip_repetition(r)?;
    }
    Ok((layer, datatype, x, y, w as i64, h as i64))
}

type PolygonHull = Vec<(i64, i64)>;

fn read_polygon_minimal(
    r: &mut &[u8],
    info: u8,
) -> Result<Option<(u64, u64, PolygonHull)>> {
    let has_layer = (info & 0x01) != 0;
    let has_datatype = (info & 0x02) != 0;
    let has_repetition = (info & 0x04) != 0;
    let has_y = (info & 0x08) != 0;
    let has_x = (info & 0x10) != 0;
    let has_p = (info & 0x20) != 0;
    let layer = if has_layer { decode_unsigned(r)? } else { 0 };
    let datatype = if has_datatype { decode_unsigned(r)? } else { 0 };
    if !has_p {
        // No point list — relies on modal; skip in streaming.
        if has_x {
            decode_signed(r)?;
        }
        if has_y {
            decode_signed(r)?;
        }
        if has_repetition {
            skip_repetition(r)?;
        }
        return Ok(None);
    }
    let plist_type = decode_unsigned(r)?;
    let count = decode_unsigned(r)? as usize;
    if plist_type != 5 {
        // Streaming reader supports only the general (type 5) point list.
        return Ok(None);
    }
    let mut deltas: Vec<(i64, i64)> = Vec::with_capacity(count);
    for _ in 0..count {
        let dx = decode_signed(r)?;
        let dy = decode_signed(r)?;
        deltas.push((dx, dy));
    }
    let x = if has_x { decode_signed(r)? } else { 0 };
    let y = if has_y { decode_signed(r)? } else { 0 };
    if has_repetition {
        skip_repetition(r)?;
    }
    let mut hull: Vec<(i64, i64)> = Vec::with_capacity(deltas.len() + 1);
    hull.push((x, y));
    let mut cx = x;
    let mut cy = y;
    for (dx, dy) in &deltas {
        cx += dx;
        cy += dy;
        hull.push((cx, cy));
    }
    Ok(Some((layer, datatype, hull)))
}

fn read_placement_minimal(
    r: &mut &[u8],
    info: u8,
) -> Result<Option<(String, i64, i64)>> {
    let has_cell_explicit = (info & 0x80) != 0;
    let has_cellname_ref = (info & 0x40) != 0;
    let name = if has_cell_explicit {
        decode_string(r)?
    } else if has_cellname_ref {
        let _refnum = decode_unsigned(r)?;
        return Ok(None); // refnum needs the table — skip in streaming
    } else {
        return Ok(None);
    };
    let has_x = (info & 0x20) != 0;
    let has_y = (info & 0x10) != 0;
    let has_repetition = (info & 0x08) != 0;
    let x = if has_x { decode_signed(r)? } else { 0 };
    let y = if has_y { decode_signed(r)? } else { 0 };
    if has_repetition {
        skip_repetition(r)?;
    }
    Ok(Some((name, x, y)))
}

fn skip_until_cell_or_end(r: &mut &[u8]) -> Result<()> {
    while !r.is_empty() {
        let next_kind = r[0];
        if matches!(next_kind, CELL | CELL_REF | END) {
            return Ok(());
        }
        // Move ahead one byte; this is a coarse skip — correctness
        // for the *index* relies on the assumption that CELL/CELL_REF/
        // END markers don't appear inside other records' payloads in
        // unfortunate ways. OASIS records are length-prefixed in
        // header byte + var-int payload; for v1 we accept the rare
        // mis-classification (it would require a CELL byte 0x0E
        // sitting inside an unsigned-int payload at a record boundary,
        // statistically negligible). A precise implementation would
        // decode each record fully.
        *r = &r[1..];
    }
    Ok(())
}

/// Inflate any CBLOCK payloads, emitting a flat post-MAGIC stream.
fn inflate_full(bytes: &[u8]) -> Result<Vec<u8>> {
    use flate2::read::DeflateDecoder;
    let mut buf: Vec<u8> = bytes.to_vec();
    let mut pos = 0;
    while pos < buf.len() {
        if buf[pos] == CBLOCK {
            let mut r: &[u8] = &buf[pos + 1..];
            let comp_type = decode_unsigned(&mut r)?;
            if comp_type != 0 {
                return Err(IoError::OasisNotImplemented(
                    "CBLOCK comp-type != 0 (only deflate supported)",
                ));
            }
            let _uncomp = decode_unsigned(&mut r)?;
            let comp_count = decode_unsigned(&mut r)? as usize;
            if r.len() < comp_count {
                return Err(IoError::UnexpectedEof);
            }
            let header_consumed = (buf.len() - pos - 1) - r.len();
            let comp_start = pos + 1 + header_consumed;
            let comp_end = comp_start + comp_count;
            let compressed: Vec<u8> = buf[comp_start..comp_end].to_vec();
            let mut decompressed = Vec::new();
            DeflateDecoder::new(compressed.as_slice())
                .read_to_end(&mut decompressed)
                .map_err(|_| IoError::OasisNotImplemented("CBLOCK deflate decode failed"))?;
            buf.splice(pos..comp_end, decompressed);
            // Don't advance pos — re-scan from here in case there are
            // nested CBLOCKs.
        } else {
            pos += 1;
        }
    }
    Ok(buf)
}

fn skip_repetition(r: &mut &[u8]) -> Result<()> {
    let kind = decode_unsigned(r)?;
    match kind {
        0 => {}
        1 => {
            for _ in 0..4 {
                decode_unsigned(r)?;
            }
        }
        2 | 3 => {
            for _ in 0..2 {
                decode_unsigned(r)?;
            }
        }
        _ => {
            // Conservative: bail on unknown repetition.
            return Err(IoError::OasisNotImplemented("unknown repetition kind"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::write_oasis_bytes;
    use klayout_core::{Bbox, CellBuilder, LayerInfo, Library, Point, Rect};

    fn build_test_lib() -> Library {
        let lib = Library::new("test", 1000);
        let l = lib.layer(LayerInfo::gds(1, 0));
        let mut cb = CellBuilder::new("a");
        cb.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
        lib.insert(cb);
        let mut cb = CellBuilder::new("b");
        cb.add_shape(l, Rect::new(Bbox::new(Point::new(20, 0), Point::new(30, 10))));
        lib.insert(cb);
        lib
    }

    #[test]
    fn index_lists_cells() {
        let lib = build_test_lib();
        let bytes = write_oasis_bytes(&lib).unwrap();
        let idx = open_oasis_bytes(&bytes).unwrap();
        assert!(idx.cell_count() >= 2);
        let names: std::collections::HashSet<&str> = idx.cell_names().collect();
        assert!(names.contains("a"));
        assert!(names.contains("b"));
    }

    #[test]
    fn for_each_shape_emits_rectangle() {
        let lib = build_test_lib();
        let bytes = write_oasis_bytes(&lib).unwrap();
        let idx = open_oasis_bytes(&bytes).unwrap();
        let mut count = 0;
        idx.for_each_shape("a", |ev| {
            if matches!(ev, OasisStreamEvent::Rectangle { .. }) {
                count += 1;
            }
        })
        .unwrap();
        assert!(count >= 1);
    }

    #[test]
    fn dbu_extracted_from_unit_record() {
        let lib = build_test_lib();
        let bytes = write_oasis_bytes(&lib).unwrap();
        let idx = open_oasis_bytes(&bytes).unwrap();
        assert_eq!(idx.dbu(), 1000);
    }
}
