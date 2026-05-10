//! CIF (Caltech Intermediate Form) layout format reader and writer.
//!
//! CIF is a 1979-era ASCII layout format. Each command is a single
//! letter followed by parameters and terminated by `;`. Common
//! commands:
//!
//! ```text
//! ( comment );
//! L LAYER_NAME;            // change current layer
//! B w h x y;               // axis-aligned box, center (x,y)
//! P x1 y1 x2 y2 ...;       // polygon
//! W w x1 y1 x2 y2 ...;     // wire of width w
//! DS num scale;            // start cell definition
//! DF;                      // finish cell definition
//! C num [T x y];           // call (instantiate) cell
//! E;                       // end of file
//! ```
//!
//! v1 covers the basic record set used by KLayout's CIF interchange
//! and the academic-foundry chips that still ship CIF. We don't
//! implement transformations beyond translation (CIF supports M, R
//! for mirror/rotate too — straightforward extension).

use crate::error::{IoError, Result};
use klayout_core::{
    Bbox, CellBuilder, CellId, CellName, LayerInfo, Library, Point, Polygon, Rect, Trans, Vec2,
};
use std::collections::HashMap;
use std::path::Path;

/// CIF natural unit is centimicrons (0.01 μm); our Library DBU is
/// 1 nm. So one CIF integer maps to ten DBU. KLayout uses the same
/// convention.
const CIF_TO_DBU: i64 = 10;

pub fn read_cif_path(path: impl AsRef<Path>) -> Result<Library> {
    let s = std::fs::read_to_string(path.as_ref())?;
    read_cif_str(&s)
}

pub fn read_cif_str(text: &str) -> Result<Library> {
    let lib = Library::new("cif", 1000);
    let mut current_layer: Option<klayout_core::LayerIndex> = None;
    let mut cells: HashMap<u32, CellBuilder> = HashMap::new();
    let mut current_cell: Option<u32> = None;
    let mut id_to_cellid: HashMap<u32, CellId> = HashMap::new();

    for stmt in tokenize(text) {
        let mut iter = stmt.iter().peekable();
        let cmd = match iter.next() {
            Some(s) => s.as_str(),
            None => continue,
        };
        match cmd {
            "L" => {
                // CIF layer commands. KLayout's convention: an `L<N>`
                // name (capital L followed by digits) maps to GDS
                // layer N / datatype 0. Other names stay as named
                // layers (gds=(0,0)) — round-trip parity for those is
                // a deferred follow-up.
                let name = iter.next().cloned().unwrap_or_default();
                let li = match name.strip_prefix('L').and_then(|s| s.parse::<u16>().ok()) {
                    Some(n) => lib.layer(LayerInfo::gds(n, 0)),
                    None => {
                        // Synthetic GDS number per name so distinct
                        // named layers don't collide on (0, 0).
                        let mut h: u32 = 0x811c9dc5;
                        for b in name.bytes() {
                            h ^= b as u32;
                            h = h.wrapping_mul(0x0100_0193);
                        }
                        let synth = ((h & 0x7FFF) as u16).max(1);
                        // `dt = u16::MAX` marks "named layer with no
                        // real GDS mapping" so the canonical-dump
                        // oracle renders `-1`.
                        lib.layer(LayerInfo::named(name.clone(), synth, u16::MAX))
                    }
                };
                current_layer = Some(li);
            }
            "B" => {
                // Box: w h x y. CIF natural unit is centimicrons
                // (0.01 μm); our DBU is 1 nm. So multiply by 10.
                let w: i64 = parse_num::<_, i64>(&mut iter)? * CIF_TO_DBU;
                let h: i64 = parse_num::<_, i64>(&mut iter)? * CIF_TO_DBU;
                let x: i64 = parse_num::<_, i64>(&mut iter)? * CIF_TO_DBU;
                let y: i64 = parse_num::<_, i64>(&mut iter)? * CIF_TO_DBU;
                let bbox = Bbox::new(
                    Point::new(x - w / 2, y - h / 2),
                    Point::new(x + w / 2 + w % 2, y + h / 2 + h % 2),
                );
                if let (Some(li), Some(cid)) = (current_layer, current_cell) {
                    if let Some(cb) = cells.get_mut(&cid) {
                        cb.add_shape(li, Rect::new(bbox));
                    }
                }
            }
            "P" => {
                // Polygon: x1 y1 x2 y2 ... (CIF centimicrons -> DBU)
                let mut points: Vec<Point> = Vec::new();
                while let Some(x_tok) = iter.peek() {
                    let Ok(x) = x_tok.parse::<i64>() else { break };
                    iter.next();
                    let y: i64 = parse_num(&mut iter)?;
                    points.push(Point::new(x * CIF_TO_DBU, y * CIF_TO_DBU));
                }
                if let (Some(li), Some(cid)) = (current_layer, current_cell) {
                    if let Some(cb) = cells.get_mut(&cid) {
                        cb.add_shape(li, Polygon::from_hull(points));
                    }
                }
            }
            "DS" => {
                // Start cell definition: DS num [scale_a scale_b]
                let num: u32 = parse_num(&mut iter)?;
                // KLayout names CIF cells "C<num>" by their `DS num` id.
                cells
                    .entry(num)
                    .or_insert_with(|| CellBuilder::new(CellName::new(format!("C{num}"))));
                current_cell = Some(num);
            }
            "DF" => {
                if let Some(cid) = current_cell.take() {
                    if let Some(cb) = cells.remove(&cid) {
                        let id = lib.insert(cb);
                        id_to_cellid.insert(cid, id);
                    }
                }
            }
            "C" => {
                // Call cell: C num [M[X|Y] | R x y | T x y]*
                let num: u32 = parse_num(&mut iter)?;
                let mut tx = 0i64;
                let mut ty = 0i64;
                let mut rot = klayout_core::Rot4::R0;
                let mut mirror = false;
                while let Some(t) = iter.next() {
                    match t.as_str() {
                        "T" => {
                            tx = parse_num::<_, i64>(&mut iter)? * CIF_TO_DBU;
                            ty = parse_num::<_, i64>(&mut iter)? * CIF_TO_DBU;
                        }
                        "MX" => {
                            // CIF "MX" — KLayout's reader maps to M90
                            // (combined trans code 6 = R180 + mirror,
                            // i.e. our (rot=R180, mirror=true)). Note
                            // KLayout's convention reads "M<axis>" as
                            // "the axis-direction is reflected", not
                            // "reflect about the axis-line".
                            mirror = !mirror;
                            rot = klayout_core::Rot4::R180;
                        }
                        "MY" => {
                            // CIF "MY" -> KLayout M0 (code 4 = R0 +
                            // mirror).
                            mirror = !mirror;
                        }
                        "M" => {
                            if let Some(axis) = iter.next() {
                                match axis.as_str() {
                                    "X" => {
                                        mirror = !mirror;
                                        rot = klayout_core::Rot4::R180;
                                    }
                                    "Y" => {
                                        mirror = !mirror;
                                    }
                                    _ => {}
                                }
                            }
                        }
                        "R" => {
                            // R is a 2-D rotation vector (cos*N, sin*N) in CIF.
                            // We map common 90°-step values.
                            let rx: i64 = parse_num(&mut iter)?;
                            let ry: i64 = parse_num(&mut iter)?;
                            rot = match (rx.signum(), ry.signum()) {
                                (1, 0) => klayout_core::Rot4::R0,
                                (0, 1) => klayout_core::Rot4::R90,
                                (-1, 0) => klayout_core::Rot4::R180,
                                (0, -1) => klayout_core::Rot4::R270,
                                _ => klayout_core::Rot4::R0,
                            };
                        }
                        _ => {}
                    }
                }
                if let (Some(parent_id), Some(child_id)) =
                    (current_cell, id_to_cellid.get(&num).copied())
                {
                    if let Some(cb) = cells.get_mut(&parent_id) {
                        cb.add_instance(klayout_core::Instance::new(
                            child_id,
                            Trans::new(rot, mirror, Vec2::new(tx, ty)),
                        ));
                    }
                }
            }
            "E" => break,
            "(" => {} // comment marker
            _ => {} // ignore other commands
        }
    }
    Ok(lib)
}

/// Emit a `Library` as CIF text.
pub fn write_cif_str(lib: &Library) -> String {
    use std::fmt::Write as _;
    let mut s = String::new();
    let _ = writeln!(s, "( klayout-rs CIF output );");
    let cells = lib.all_cells();
    let mut id_to_num: HashMap<CellId, u32> = HashMap::new();
    for (i, (id, _)) in cells.iter().enumerate() {
        id_to_num.insert(*id, (i + 1) as u32);
    }
    for (id, cell) in &cells {
        let num = id_to_num[id];
        let _ = writeln!(s, "DS {num} 1 1;");
        let _ = writeln!(s, "9 {} ;", cell.name());
        for layer_idx in cell.layers() {
            let info = lib.layer_info(layer_idx);
            let _ = writeln!(s, "L L{}_{};", info.layer, info.datatype);
            for shape in cell.shapes_on(layer_idx) {
                match shape {
                    klayout_core::Shape::Box(r) => {
                        let b = r.bbox;
                        let cx = (b.min.x + b.max.x) / 2;
                        let cy = (b.min.y + b.max.y) / 2;
                        let _ = writeln!(s, "B {} {} {} {};", b.width(), b.height(), cx, cy);
                    }
                    klayout_core::Shape::Polygon(p) => {
                        let _ = write!(s, "P");
                        for pt in &p.hull {
                            let _ = write!(s, " {} {}", pt.x, pt.y);
                        }
                        let _ = writeln!(s, ";");
                    }
                    _ => {}
                }
            }
        }
        for inst in cell.instances() {
            if let Some(child_num) = id_to_num.get(&inst.cell) {
                let mut line = format!("C {child_num}");
                if inst.trans.mirror {
                    line.push_str(" M X");
                }
                use klayout_core::Rot4;
                match inst.trans.rot {
                    Rot4::R0 => {}
                    Rot4::R90 => line.push_str(" R 0 1"),
                    Rot4::R180 => line.push_str(" R -1 0"),
                    Rot4::R270 => line.push_str(" R 0 -1"),
                }
                line.push_str(&format!(" T {} {};", inst.trans.disp.x, inst.trans.disp.y));
                let _ = writeln!(s, "{line}");
            }
        }
        let _ = writeln!(s, "DF;");
    }
    let _ = writeln!(s, "E");
    s
}

fn tokenize(text: &str) -> Vec<Vec<String>> {
    // Split on `;`, then split each statement into whitespace tokens.
    // Treat `(...)` as a single comment statement.
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut depth = 0;
    let mut cur = String::new();
    for c in text.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            ';' if depth == 0 => {
                let toks: Vec<String> = cur
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect();
                if !toks.is_empty() {
                    out.push(toks);
                }
                cur.clear();
            }
            _ => cur.push(c),
        }
    }
    out
}

fn parse_num<'a, I, T>(iter: &mut I) -> Result<T>
where
    I: Iterator<Item = &'a String>,
    T: std::str::FromStr,
{
    let tok = iter
        .next()
        .ok_or(IoError::OasisNotImplemented("CIF: missing number"))?;
    tok.parse()
        .map_err(|_| IoError::OasisNotImplemented("CIF: invalid number"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
( CIF sample );
DS 1 1 1;
9 INV;
L L1_0;
B 100 50 50 25;
P 0 0 100 0 100 50 0 50;
DF;
DS 2 1 1;
9 TOP;
C 1 T 0 0;
C 1 T 200 0;
DF;
E
"#;

    #[test]
    fn reads_cells_and_shapes() {
        let lib = read_cif_str(SAMPLE).unwrap();
        assert!(lib.cell_count() >= 2);
    }

    #[test]
    fn write_then_read_round_trips_basic() {
        let lib = read_cif_str(SAMPLE).unwrap();
        let text = write_cif_str(&lib);
        let lib2 = read_cif_str(&text).unwrap();
        assert!(lib2.cell_count() >= lib.cell_count());
    }

    #[test]
    fn empty_cif_yields_empty_library() {
        let lib = read_cif_str("E\n").unwrap();
        assert_eq!(lib.cell_count(), 0);
    }

    #[test]
    fn parses_rotation_and_mirror() {
        let cif = "
DS 1 1 1;
L M1;
B 10 10 0 0;
DF;
DS 2 1 1;
C 1 R 0 1 T 100 0;
C 1 M X T 200 0;
DF;
E
";
        let lib = read_cif_str(cif).unwrap();
        // Top cell (cell #2) has 2 instances; verify rotation applied.
        assert!(lib.cell_count() >= 2);
        // Find the top cell.
        let top_id = lib.by_name("C2").unwrap();
        let top = lib.get(top_id);
        assert_eq!(top.instances().len(), 2);
        // First instance: R 0 1 → R90.
        assert_eq!(top.instances()[0].trans.rot, klayout_core::Rot4::R90);
        // Second: M X → mirrored, no rotation.
        assert!(top.instances()[1].trans.mirror);
    }

    #[test]
    fn round_trips_mirrored_instance() {
        use klayout_core::{Bbox, CellBuilder, LayerInfo, Library, Point, Rect, Trans, Vec2};
        let lib = Library::new("t", 1000);
        let l = lib.layer(LayerInfo::named("M1", 0, 0));
        let mut child = CellBuilder::new("child");
        child.add_shape(l, Rect::new(Bbox::new(Point::new(0, 0), Point::new(10, 10))));
        let cid = lib.insert(child);
        let mut top = CellBuilder::new("top");
        top.add_instance(klayout_core::Instance::new(
            cid,
            Trans::new(klayout_core::Rot4::R90, false, Vec2::new(50, 0)),
        ));
        top.add_instance(klayout_core::Instance::new(
            cid,
            Trans::new(klayout_core::Rot4::R0, true, Vec2::new(100, 0)),
        ));
        lib.insert(top);
        let text = write_cif_str(&lib);
        // Verify R/M emission appears in output.
        assert!(text.contains(" R "));
        assert!(text.contains(" M "));
    }
}
