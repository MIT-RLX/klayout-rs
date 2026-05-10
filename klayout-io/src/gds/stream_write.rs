//! Streaming GDS writer.
//!
//! The full writer (`write_gds_bytes`) materialises the entire output
//! in memory, which doubles peak memory at write time. For huge
//! designs you'd rather emit cell-by-cell to a `Write` sink (file
//! buffered I/O, network stream, compressor pipe).
//!
//! [`StreamingGdsWriter`] takes an `impl Write` and lets you push
//! cells in any order with the existing `klayout_core::Cell` data
//! types. Library framing (HEADER / BGNLIB / UNITS / ENDLIB) is
//! handled by `start()` / `finish()`. Cells in between are emitted
//! by `write_cell()`.
//!
//! Pre-condition: cells must be added in topo order (children before
//! parents) — GDS readers tolerate the reverse via topo-sort but
//! emitting that way matches KLayout's convention and keeps the
//! output stream forward-readable. Caller picks the order.

use super::codec::*;
use super::records::*;
use crate::error::{IoError, Result};
use klayout_core::{CellId, Library, Path as CorePath, Rect, Shape};
use std::collections::HashMap;
use std::io::Write;

pub struct StreamingGdsWriter<W: Write> {
    out: W,
    /// Map from CellId already emitted to its cell name (for SREF/AREF).
    name_for: HashMap<CellId, String>,
    started: bool,
}

impl<W: Write> StreamingGdsWriter<W> {
    pub fn new(out: W) -> Self {
        Self {
            out,
            name_for: HashMap::new(),
            started: false,
        }
    }

    /// Write the library header. Must be called before any `write_cell`.
    pub fn start(&mut self, library_name: &str, dbu: i64) -> Result<()> {
        write_record_i16(&mut self.out, HEADER, &[600])?;
        write_record_i16(&mut self.out, BGNLIB, &[0i16; 12])?;
        write_record_str(&mut self.out, LIBNAME, library_name)?;
        let user_unit = 1.0e-3;
        let dbu_meters = 1e-6 / dbu as f64;
        write_record_f64(&mut self.out, UNITS, &[user_unit, dbu_meters])?;
        self.started = true;
        Ok(())
    }

    /// Write one cell. The library reference is needed to resolve
    /// child-cell names for SREF/AREF emission.
    pub fn write_cell(&mut self, lib: &Library, cell_id: CellId) -> Result<()> {
        if !self.started {
            return Err(IoError::MissingRecord("BGNLIB"));
        }
        let cell = lib.get(cell_id);
        write_record_i16(&mut self.out, BGNSTR, &[0i16; 12])?;
        write_record_str(&mut self.out, STRNAME, cell.name().as_str())?;

        for layer_idx in cell.layers() {
            let info = lib.layer_info(layer_idx);
            for shape in cell.shapes_on(layer_idx) {
                self.emit_shape(info.layer, info.datatype, shape)?;
            }
        }

        for inst in cell.instances() {
            let name = self
                .name_for
                .get(&inst.cell)
                .cloned()
                .unwrap_or_else(|| lib.get(inst.cell).name().as_str().to_string());
            self.emit_instance(&name, inst)?;
        }

        write_record(&mut self.out, ENDSTR, &[])?;
        self.name_for
            .insert(cell_id, cell.name().as_str().to_string());
        Ok(())
    }

    /// Emit ENDLIB and consume the writer.
    pub fn finish(mut self) -> Result<W> {
        write_record(&mut self.out, ENDLIB, &[])?;
        self.out.flush()?;
        Ok(self.out)
    }

    fn emit_shape(&mut self, layer: u16, datatype: u16, shape: &Shape) -> Result<()> {
        match shape {
            Shape::Polygon(p) => emit_boundary(&mut self.out, layer, datatype, &p.hull),
            Shape::Box(r) => emit_box(&mut self.out, layer, datatype, *r),
            Shape::Path(p) => emit_path(&mut self.out, layer, datatype, p),
            Shape::Text(t) => emit_text(&mut self.out, layer, datatype, t),
        }
    }

    fn emit_instance(&mut self, name: &str, inst: &klayout_core::Instance) -> Result<()> {
        emit_sref(&mut self.out, name, inst)
    }
}

fn emit_boundary<W: Write>(
    out: &mut W,
    layer: u16,
    datatype: u16,
    hull: &[klayout_core::Point],
) -> Result<()> {
    write_record(out, BOUNDARY, &[])?;
    write_record_i16(out, LAYER, &[layer as i16])?;
    write_record_i16(out, DATATYPE, &[datatype as i16])?;
    let mut xy: Vec<i32> = Vec::with_capacity(hull.len() * 2 + 2);
    for p in hull {
        xy.push(p.x as i32);
        xy.push(p.y as i32);
    }
    if !hull.is_empty() {
        xy.push(hull[0].x as i32);
        xy.push(hull[0].y as i32);
    }
    write_record_i32(out, XY, &xy)?;
    write_record(out, ENDEL, &[])?;
    Ok(())
}

fn emit_box<W: Write>(out: &mut W, layer: u16, datatype: u16, r: Rect) -> Result<()> {
    write_record(out, BOX_REC, &[])?;
    write_record_i16(out, LAYER, &[layer as i16])?;
    write_record_i16(out, BOXTYPE, &[datatype as i16])?;
    let b = r.bbox;
    let xy: Vec<i32> = vec![
        b.min.x as i32,
        b.min.y as i32,
        b.max.x as i32,
        b.min.y as i32,
        b.max.x as i32,
        b.max.y as i32,
        b.min.x as i32,
        b.max.y as i32,
        b.min.x as i32,
        b.min.y as i32,
    ];
    write_record_i32(out, XY, &xy)?;
    write_record(out, ENDEL, &[])?;
    Ok(())
}

fn emit_path<W: Write>(
    out: &mut W,
    layer: u16,
    datatype: u16,
    p: &CorePath,
) -> Result<()> {
    use klayout_core::PathCap;
    write_record(out, PATH, &[])?;
    write_record_i16(out, LAYER, &[layer as i16])?;
    write_record_i16(out, DATATYPE, &[datatype as i16])?;
    let path_type: i16 = match p.cap {
        PathCap::Flat => 0,
        PathCap::Round => 1,
        PathCap::Extended => 2,
    };
    write_record_i16(out, PATHTYPE, &[path_type])?;
    write_record_i32(out, WIDTH, &[p.width as i32])?;
    if p.begin_ext != 0 {
        write_record_i32(out, BGNEXTN, &[p.begin_ext as i32])?;
    }
    if p.end_ext != 0 {
        write_record_i32(out, ENDEXTN, &[p.end_ext as i32])?;
    }
    let mut xy: Vec<i32> = Vec::with_capacity(p.points.len() * 2);
    for pt in &p.points {
        xy.push(pt.x as i32);
        xy.push(pt.y as i32);
    }
    write_record_i32(out, XY, &xy)?;
    write_record(out, ENDEL, &[])?;
    Ok(())
}

fn emit_text<W: Write>(
    out: &mut W,
    layer: u16,
    datatype: u16,
    t: &klayout_core::Text,
) -> Result<()> {
    write_record(out, TEXT, &[])?;
    write_record_i16(out, LAYER, &[layer as i16])?;
    write_record_i16(out, TEXTTYPE, &[datatype as i16])?;
    let xy = vec![t.anchor.x as i32, t.anchor.y as i32];
    write_record_i32(out, XY, &xy)?;
    write_record_str(out, STRING, t.string.as_str())?;
    write_record(out, ENDEL, &[])?;
    Ok(())
}

fn emit_sref<W: Write>(
    out: &mut W,
    name: &str,
    inst: &klayout_core::Instance,
) -> Result<()> {
    use klayout_core::Rot4;
    write_record(out, SREF, &[])?;
    write_record_str(out, SNAME, name)?;
    let needs = inst.trans.mirror || inst.trans.rot != Rot4::R0;
    if needs {
        let strans_bits: u16 = if inst.trans.mirror { 0x8000 } else { 0 };
        write_record_bits(out, STRANS, strans_bits)?;
        let angle = match inst.trans.rot {
            Rot4::R0 => 0.0,
            Rot4::R90 => 90.0,
            Rot4::R180 => 180.0,
            Rot4::R270 => 270.0,
        };
        if angle != 0.0 {
            write_record_f64(out, ANGLE, &[angle])?;
        }
    }
    let xy = vec![inst.trans.disp.x as i32, inst.trans.disp.y as i32];
    write_record_i32(out, XY, &xy)?;
    write_record(out, ENDEL, &[])?;
    Ok(())
}

// ---- record-level helpers (mirror write.rs but generic over Write) ----

fn write_record<W: Write>(out: &mut W, kind: u16, data: &[u8]) -> Result<()> {
    let total: u16 = 4 + data.len() as u16;
    out.write_all(&total.to_be_bytes())?;
    out.write_all(&kind.to_be_bytes())?;
    out.write_all(data)?;
    if data.len() % 2 != 0 {
        out.write_all(&[0])?;
    }
    Ok(())
}

fn write_record_i16<W: Write>(out: &mut W, kind: u16, vals: &[i16]) -> Result<()> {
    let mut data: Vec<u8> = Vec::with_capacity(vals.len() * 2);
    for v in vals {
        data.extend_from_slice(&v.to_be_bytes());
    }
    write_record(out, kind, &data)
}

fn write_record_i32<W: Write>(out: &mut W, kind: u16, vals: &[i32]) -> Result<()> {
    let mut data: Vec<u8> = Vec::with_capacity(vals.len() * 4);
    for v in vals {
        data.extend_from_slice(&v.to_be_bytes());
    }
    write_record(out, kind, &data)
}

fn write_record_f64<W: Write>(out: &mut W, kind: u16, vals: &[f64]) -> Result<()> {
    let mut data: Vec<u8> = Vec::with_capacity(vals.len() * 8);
    for v in vals {
        data.extend_from_slice(&gds_f64_encode(*v));
    }
    write_record(out, kind, &data)
}

fn write_record_str<W: Write>(out: &mut W, kind: u16, s: &str) -> Result<()> {
    let mut data: Vec<u8> = s.as_bytes().to_vec();
    if data.len() % 2 != 0 {
        data.push(0);
    }
    write_record(out, kind, &data)
}

fn write_record_bits<W: Write>(out: &mut W, kind: u16, bits: u16) -> Result<()> {
    let data = bits.to_be_bytes();
    write_record(out, kind, &data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read_gds_bytes;
    use klayout_core::{Bbox, CellBuilder, LayerInfo, Library, Point, Rect};

    #[test]
    fn write_roundtrips_via_streaming() {
        let lib = Library::new("test", 1000);
        let l = lib.layer(LayerInfo::gds(1, 0));
        let mut cb = CellBuilder::new("a");
        cb.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
        let id = lib.insert(cb);

        let mut buf: Vec<u8> = Vec::new();
        let mut w = StreamingGdsWriter::new(&mut buf);
        w.start("test", 1000).unwrap();
        w.write_cell(&lib, id).unwrap();
        w.finish().unwrap();

        let lib2 = read_gds_bytes(&buf).unwrap();
        assert_eq!(lib2.cell_count(), 1);
        assert!(lib2.by_name("a").is_some());
    }

    #[test]
    fn multiple_cells_in_topo_order() {
        let lib = Library::new("test", 1000);
        let l = lib.layer(LayerInfo::gds(1, 0));
        let mut a_cb = CellBuilder::new("a");
        a_cb.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(5, 5))));
        let a = lib.insert(a_cb);
        let mut b_cb = CellBuilder::new("b");
        b_cb.add_shape(l, Rect::new(Bbox::new(Point::new(10, 0), Point::new(20, 10))));
        b_cb.add_instance(klayout_core::Instance::new(a, klayout_core::Trans::IDENTITY));
        let b = lib.insert(b_cb);

        let mut buf: Vec<u8> = Vec::new();
        let mut w = StreamingGdsWriter::new(&mut buf);
        w.start("test", 1000).unwrap();
        w.write_cell(&lib, a).unwrap();
        w.write_cell(&lib, b).unwrap();
        w.finish().unwrap();

        let lib2 = read_gds_bytes(&buf).unwrap();
        assert_eq!(lib2.cell_count(), 2);
        let b2 = lib2.get(lib2.by_name("b").unwrap());
        assert_eq!(b2.instances().len(), 1);
    }

    #[test]
    fn empty_library_roundtrips() {
        let mut buf: Vec<u8> = Vec::new();
        let mut w = StreamingGdsWriter::new(&mut buf);
        w.start("empty", 500).unwrap();
        w.finish().unwrap();
        let lib2 = read_gds_bytes(&buf).unwrap();
        assert_eq!(lib2.cell_count(), 0);
        assert_eq!(lib2.dbu(), 500);
    }
}
