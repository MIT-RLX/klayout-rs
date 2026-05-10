//! GDSII reader: bytes → `Library`.
//!
//! Two-phase: parse to an in-memory `ParsedLib`, then convert to `Library`
//! by topological sort over SREF/AREF references. This handles GDS files
//! where children appear after parents (legal but uncommon).

use super::super::error::{IoError, Result};
use super::codec::*;
use super::records::*;
use klayout_core::{
    Bbox, CellBuilder, CellId, CellName, HAlign, Instance, LayerInfo, Library, Path as CorePath,
    PathCap, Point, Polygon, Properties, PropertyValue, Rect, Repetition, Rot4, Text, Trans, VAlign,
    Vec2,
};
use std::collections::HashMap;
use std::path::Path;

pub fn read_gds_path(path: impl AsRef<Path>) -> Result<Library> {
    let bytes = std::fs::read(path.as_ref())?;
    read_gds_bytes(&bytes)
}

pub fn read_gds_bytes(bytes: &[u8]) -> Result<Library> {
    let parsed = parse(bytes)?;
    convert(parsed)
}

// ---------- ParsedLib intermediate ----------

#[derive(Debug, Default)]
struct ParsedLib {
    name: String,
    user_unit: f64,
    dbu_meters: f64,
    cells: Vec<ParsedCell>,
}

#[derive(Debug, Default)]
struct ParsedCell {
    name: String,
    elements: Vec<ParsedElement>,
    /// Cell-level PROPATTR/PROPVALUE pairs that appear inside BGNSTR
    /// before the first element (a non-standard but supported KLayout
    /// extension).
    properties: Vec<(u16, String)>,
}

#[derive(Debug)]
enum ParsedElement {
    Boundary {
        layer: u16,
        datatype: u16,
        points: Vec<(i32, i32)>,
    },
    Path {
        layer: u16,
        datatype: u16,
        width: i32,
        path_type: u16,
        points: Vec<(i32, i32)>,
        bgn_ext: i32,
        end_ext: i32,
    },
    Box {
        layer: u16,
        datatype: u16,
        points: Vec<(i32, i32)>,
    },
    Sref {
        sname: String,
        strans: u16,
        angle: f64,
        mag: f64,
        origin: (i32, i32),
        properties: Vec<(u16, String)>,
    },
    Aref {
        sname: String,
        strans: u16,
        angle: f64,
        mag: f64,
        n_cols: u16,
        n_rows: u16,
        origin: (i32, i32),
        col_end: (i32, i32),
        row_end: (i32, i32),
        properties: Vec<(u16, String)>,
    },
    Text {
        layer: u16,
        texttype: u16,
        position: (i32, i32),
        string: String,
    },
}

// ---------- Phase 1: parse ----------

fn parse(bytes: &[u8]) -> Result<ParsedLib> {
    let mut r = ByteReader::new(bytes);
    let mut lib = ParsedLib::default();
    let mut seen_header = false;
    let mut in_lib = false;

    while let Some(h) = r.read_record_header()? {
        let data = r.take(h.data_len())?;
        match h.kind {
            HEADER => {
                seen_header = true;
            }
            BGNLIB => {
                in_lib = true;
            }
            LIBNAME => {
                lib.name = parse_string(data);
            }
            UNITS => {
                let f = parse_f64_array(data);
                if f.len() != 2 {
                    return Err(IoError::MalformedUnits);
                }
                lib.user_unit = f[0];
                lib.dbu_meters = f[1];
            }
            BGNSTR => {
                let cell = parse_structure(&mut r)?;
                lib.cells.push(cell);
            }
            ENDLIB => {
                if !seen_header || !in_lib {
                    return Err(IoError::MissingRecord("HEADER/BGNLIB"));
                }
                return Ok(lib);
            }
            _ => {
                // Tolerate unknown library-level records.
            }
        }
    }
    Err(IoError::MissingRecord("ENDLIB"))
}

fn parse_structure(r: &mut ByteReader<'_>) -> Result<ParsedCell> {
    // BGNSTR data already consumed by caller. Read STRNAME, then elements.
    let mut cell = ParsedCell::default();
    let h = r
        .read_record_header()?
        .ok_or(IoError::UnexpectedEof)?;
    if h.kind != STRNAME {
        return Err(IoError::UnexpectedRecord {
            kind: h.kind,
            context: "after BGNSTR",
        });
    }
    cell.name = parse_string(r.take(h.data_len())?);

    // Cell-level PROPATTR/PROPVALUE pairs are accumulated until we hit
    // the first element record. (Non-standard but a common extension —
    // KLayout writes them this way for cell metadata.)
    let mut pending_attr: Option<u16> = None;

    loop {
        let h = r
            .read_record_header()?
            .ok_or(IoError::UnexpectedEof)?;
        let data = r.take(h.data_len())?;
        match h.kind {
            ENDSTR => {
                return Ok(cell);
            }
            PROPATTR => {
                pending_attr = parse_i16_array(data).first().map(|v| *v as u16);
            }
            PROPVALUE => {
                if let Some(attr) = pending_attr.take() {
                    cell.properties.push((attr, parse_string(data)));
                }
            }
            BOUNDARY => {
                let el = parse_boundary(r)?;
                cell.elements.push(el);
            }
            PATH => {
                let el = parse_path(r)?;
                cell.elements.push(el);
            }
            BOX_REC => {
                let el = parse_box(r)?;
                cell.elements.push(el);
            }
            SREF => {
                let el = parse_sref(r)?;
                cell.elements.push(el);
            }
            AREF => {
                let el = parse_aref(r)?;
                cell.elements.push(el);
            }
            TEXT => {
                let el = parse_text(r)?;
                cell.elements.push(el);
            }
            _ => {
                // Ignore unknown records.
            }
        }
    }
}

struct ElementHeaders {
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
    properties: Vec<(u16, String)>,
    pending_propattr: Option<u16>,
}

impl Default for ElementHeaders {
    fn default() -> Self {
        Self {
            layer: 0,
            datatype: 0,
            width: 0,
            path_type: 0,
            bgn_ext: 0,
            end_ext: 0,
            xy: Vec::new(),
            sname: String::new(),
            strans: 0,
            angle: 0.0,
            mag: 1.0,
            n_cols: 0,
            n_rows: 0,
            string: String::new(),
            properties: Vec::new(),
            pending_propattr: None,
        }
    }
}

fn read_until_endel(r: &mut ByteReader<'_>) -> Result<ElementHeaders> {
    let mut h = ElementHeaders::default();
    loop {
        let rh = r
            .read_record_header()?
            .ok_or(IoError::UnexpectedEof)?;
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
                if data.len() >= 2 {
                    h.strans = u16::from_be_bytes([data[0], data[1]]);
                }
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
                h.pending_propattr =
                    parse_i16_array(data).first().map(|v| *v as u16);
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

fn parse_boundary(r: &mut ByteReader<'_>) -> Result<ParsedElement> {
    let h = read_until_endel(r)?;
    Ok(ParsedElement::Boundary {
        layer: h.layer,
        datatype: h.datatype,
        points: h.xy,
    })
}

fn parse_path(r: &mut ByteReader<'_>) -> Result<ParsedElement> {
    let h = read_until_endel(r)?;
    Ok(ParsedElement::Path {
        layer: h.layer,
        datatype: h.datatype,
        width: h.width,
        path_type: h.path_type,
        points: h.xy,
        bgn_ext: h.bgn_ext,
        end_ext: h.end_ext,
    })
}

fn parse_box(r: &mut ByteReader<'_>) -> Result<ParsedElement> {
    let h = read_until_endel(r)?;
    Ok(ParsedElement::Box {
        layer: h.layer,
        datatype: h.datatype,
        points: h.xy,
    })
}

fn parse_sref(r: &mut ByteReader<'_>) -> Result<ParsedElement> {
    let h = read_until_endel(r)?;
    let origin = h.xy.first().copied().unwrap_or((0, 0));
    Ok(ParsedElement::Sref {
        sname: h.sname,
        strans: h.strans,
        angle: h.angle,
        mag: h.mag,
        origin,
        properties: h.properties,
    })
}

fn parse_aref(r: &mut ByteReader<'_>) -> Result<ParsedElement> {
    let h = read_until_endel(r)?;
    if h.xy.len() < 3 || h.n_cols == 0 || h.n_rows == 0 {
        return Err(IoError::ZeroArefCount);
    }
    Ok(ParsedElement::Aref {
        sname: h.sname,
        strans: h.strans,
        angle: h.angle,
        mag: h.mag,
        n_cols: h.n_cols,
        n_rows: h.n_rows,
        origin: h.xy[0],
        col_end: h.xy[1],
        row_end: h.xy[2],
        properties: h.properties,
    })
}

fn parse_text(r: &mut ByteReader<'_>) -> Result<ParsedElement> {
    let h = read_until_endel(r)?;
    let pos = h.xy.first().copied().unwrap_or((0, 0));
    Ok(ParsedElement::Text {
        layer: h.layer,
        texttype: h.datatype,
        position: pos,
        string: h.string,
    })
}

// ---------- Phase 2: convert to Library ----------

fn convert(p: ParsedLib) -> Result<Library> {
    let dbu = if p.dbu_meters > 0.0 {
        let raw = (1e-6 / p.dbu_meters).round();
        if raw >= 1.0 && raw.is_finite() {
            raw as i64
        } else {
            1000
        }
    } else {
        1000
    };

    let lib = Library::new(p.name, dbu);

    // Topo sort cells by SREF/AREF dependencies. Children must be inserted first.
    let by_name: HashMap<String, usize> = p
        .cells
        .iter()
        .enumerate()
        .map(|(i, c)| (c.name.clone(), i))
        .collect();
    let order = topo_sort(&p.cells, &by_name)?;

    let mut name_to_id: HashMap<String, CellId> = HashMap::new();

    for cell_idx in order {
        let pc = &p.cells[cell_idx];
        let mut cb = CellBuilder::new(CellName::new(pc.name.clone()));
        for (attr, value) in &pc.properties {
            cb.set_property(
                attr.to_string(),
                PropertyValue::String(value.as_str().into()),
            );
        }

        for el in &pc.elements {
            match el {
                ParsedElement::Boundary {
                    layer,
                    datatype,
                    points,
                } => {
                    let li = lib.layer(LayerInfo::gds(*layer, *datatype));
                    let mut hull: Vec<Point> = points
                        .iter()
                        .map(|(x, y)| Point::new(*x as i64, *y as i64))
                        .collect();
                    if hull.len() >= 2 && hull.first() == hull.last() {
                        hull.pop();
                    }
                    // Detect keyhole-encoded polygons-with-holes (self-touching
                    // boundaries). If found, reconstruct hull + holes.
                    let (hull, holes) = split_keyholes(hull);
                    // Match KLayout: an axis-aligned 4-vertex polygon is
                    // classified as a Box on read, regardless of whether the
                    // file used a BOX or BOUNDARY record.
                    if holes.is_empty() {
                        if let Some(r) = polygon_as_rect(&hull) {
                            cb.add_shape(li, r);
                        } else {
                            cb.add_shape(li, Polygon::from_hull(hull));
                        }
                    } else {
                        let mut p = Polygon::from_hull(hull);
                        for hole_pts in holes {
                            p.add_hole(hole_pts);
                        }
                        cb.add_shape(li, p);
                    }
                }
                ParsedElement::Path {
                    layer,
                    datatype,
                    width,
                    path_type,
                    points,
                    bgn_ext,
                    end_ext,
                } => {
                    let li = lib.layer(LayerInfo::gds(*layer, *datatype));
                    let pts = points
                        .iter()
                        .map(|(x, y)| Point::new(*x as i64, *y as i64));
                    let cap = match path_type {
                        0 => PathCap::Flat,
                        1 => PathCap::Round,
                        2 | 4 => PathCap::Extended,
                        _ => PathCap::Flat,
                    };
                    let mut path = CorePath::new(pts, *width as i64);
                    path.cap = cap;
                    if *path_type == 4 {
                        path.begin_ext = *bgn_ext as i64;
                        path.end_ext = *end_ext as i64;
                    }
                    cb.add_shape(li, path);
                }
                ParsedElement::Box {
                    layer,
                    datatype,
                    points,
                } => {
                    let li = lib.layer(LayerInfo::gds(*layer, *datatype));
                    let mut bb = Bbox::EMPTY;
                    for (x, y) in points {
                        bb.expand_to(Point::new(*x as i64, *y as i64));
                    }
                    cb.add_shape(li, Rect::new(bb));
                }
                ParsedElement::Sref {
                    sname,
                    strans,
                    angle,
                    mag,
                    origin,
                    properties,
                } => {
                    if (*mag - 1.0).abs() > 1e-12 {
                        return Err(IoError::UnsupportedMagnification(*mag));
                    }
                    let trans = make_trans(*strans, *angle, *origin)?;
                    let cid = name_to_id
                        .get(sname)
                        .copied()
                        .ok_or_else(|| IoError::CellNotFound(sname.clone()))?;
                    let mut inst = Instance::new(cid, trans);
                    apply_props(&mut inst.properties, properties);
                    cb.add_instance(inst);
                }
                ParsedElement::Aref {
                    sname,
                    strans,
                    angle,
                    mag,
                    n_cols,
                    n_rows,
                    origin,
                    col_end,
                    row_end,
                    properties,
                } => {
                    if (*mag - 1.0).abs() > 1e-12 {
                        return Err(IoError::UnsupportedMagnification(*mag));
                    }
                    let trans = make_trans(*strans, *angle, *origin)?;
                    let cid = name_to_id
                        .get(sname)
                        .copied()
                        .ok_or_else(|| IoError::CellNotFound(sname.clone()))?;
                    let col_step = Vec2::new(
                        (col_end.0 - origin.0) as i64 / *n_cols as i64,
                        (col_end.1 - origin.1) as i64 / *n_cols as i64,
                    );
                    let row_step = Vec2::new(
                        (row_end.0 - origin.0) as i64 / *n_rows as i64,
                        (row_end.1 - origin.1) as i64 / *n_rows as i64,
                    );
                    let rep = Repetition::Regular {
                        col: col_step,
                        row: row_step,
                        n_rows: *n_rows as u32,
                        n_cols: *n_cols as u32,
                    };
                    let mut inst = Instance::new(cid, trans).with_repetition(rep);
                    apply_props(&mut inst.properties, properties);
                    cb.add_instance(inst);
                }
                ParsedElement::Text {
                    layer,
                    texttype,
                    position,
                    string,
                } => {
                    let li = lib.layer(LayerInfo::gds(*layer, *texttype));
                    let t = Text {
                        string: string.clone().into(),
                        anchor: Point::new(position.0 as i64, position.1 as i64),
                        size: 0,
                        halign: HAlign::Left,
                        valign: VAlign::Bottom,
                    };
                    cb.add_shape(li, t);
                }
            }
        }

        // Promote any text labels in the cell whose layer contains the
        // substring "PORT" or whose name is a port-decl convention into Ports?
        // Out of scope for v1 — labels remain shapes.

        // Promote any element on a port layer? Out of scope. Ports are not
        // round-tripped through plain GDS; we rely on properties for that.

        let id = lib.insert(cb);
        name_to_id.insert(pc.name.clone(), id);
    }

    Ok(lib)
}

fn topo_sort(
    cells: &[ParsedCell],
    by_name: &HashMap<String, usize>,
) -> Result<Vec<usize>> {
    let mut order = Vec::with_capacity(cells.len());
    let mut visited = vec![false; cells.len()];
    let mut on_stack = vec![false; cells.len()];

    for i in 0..cells.len() {
        if !visited[i] {
            visit(i, cells, by_name, &mut visited, &mut on_stack, &mut order)?;
        }
    }
    Ok(order)
}

fn visit(
    i: usize,
    cells: &[ParsedCell],
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
    for el in &cells[i].elements {
        let child_name = match el {
            ParsedElement::Sref { sname, .. } => Some(sname.as_str()),
            ParsedElement::Aref { sname, .. } => Some(sname.as_str()),
            _ => None,
        };
        if let Some(name) = child_name {
            if let Some(&j) = by_name.get(name) {
                visit(j, cells, by_name, visited, on_stack, order)?;
            } else {
                return Err(IoError::CellNotFound(name.to_string()));
            }
        }
    }
    on_stack[i] = false;
    visited[i] = true;
    order.push(i);
    Ok(())
}

/// Convert GDS PROPATTR/PROPVALUE pairs to our Properties.
/// Keys are the GDS attribute number serialized as a decimal string so
/// they round-trip through `Properties` and back to a u16 on write.
fn apply_props(out: &mut Properties, pairs: &[(u16, String)]) {
    for (attr, value) in pairs {
        out.set(format!("{}", attr), PropertyValue::String(value.clone().into()));
    }
}

/// Detect self-touching keyhole pattern in `hull` and split into a clean
/// outer hull + list of holes (in CCW winding to match our convention).
///
/// Pattern: ..., v_outer, v_hole, [hole CW walk], v_hole, v_outer, ...
/// Both `v_outer` and `v_hole` appear twice; the inner pair (v_hole) has
/// the smallest gap. Recursive — handles polygons with multiple holes.
fn split_keyholes(hull: Vec<Point>) -> (Vec<Point>, Vec<Vec<Point>>) {
    let mut current = hull;
    let mut holes = Vec::new();
    while let Some((new_hull, hole)) = extract_one_keyhole(&current) {
        current = new_hull;
        holes.push(hole);
    }
    (current, holes)
}

fn extract_one_keyhole(hull: &[Point]) -> Option<(Vec<Point>, Vec<Point>)> {
    let n = hull.len();
    if n < 6 {
        return None;
    }
    // Index pairs (i, j) with hull[i] == hull[j], j > i, and the slit
    // bracket condition hull[i-1] == hull[j+1]. Pick the innermost
    // (smallest j-i) so multi-hole keyholes are extracted hole-first.
    let mut best: Option<(usize, usize, usize)> = None; // (gap, i, j)
    for i in 1..n - 2 {
        for j in i + 2..n - 1 {
            if hull[i] == hull[j]
                && hull[i - 1] == hull[j + 1]
                && hull[i] != hull[i - 1]
            {
                let gap = j - i;
                if best.map(|(g, _, _)| gap < g).unwrap_or(true) {
                    best = Some((gap, i, j));
                }
            }
        }
    }
    let (_, i, j) = best?;
    let v_hole = hull[i];
    let loop_verts: Vec<Point> = hull[i + 1..j].to_vec();
    // CCW hole = [v_hole, ...reversed_loop].
    let mut hole = Vec::with_capacity(loop_verts.len() + 1);
    hole.push(v_hole);
    for v in loop_verts.iter().rev() {
        hole.push(*v);
    }
    // Remove slit: hull[..i-1] + hull[j+2..]. Drops both v_outer copies
    // and the v_hole pair plus the loop.
    let mut new_hull: Vec<Point> = Vec::with_capacity(n - (j - i + 3));
    new_hull.extend_from_slice(&hull[..i - 1]);
    new_hull.extend_from_slice(&hull[j + 2..]);
    Some((new_hull, hole))
}

fn polygon_as_rect(hull: &[Point]) -> Option<Rect> {
    if hull.len() != 4 {
        return None;
    }
    let mut xs: Vec<i64> = hull.iter().map(|p| p.x).collect();
    let mut ys: Vec<i64> = hull.iter().map(|p| p.y).collect();
    xs.sort();
    xs.dedup();
    ys.sort();
    ys.dedup();
    if xs.len() == 2 && ys.len() == 2 {
        Some(Rect::new(Bbox::new(
            Point::new(xs[0], ys[0]),
            Point::new(xs[1], ys[1]),
        )))
    } else {
        None
    }
}

fn make_trans(strans: u16, angle: f64, origin: (i32, i32)) -> Result<Trans> {
    let mirror = (strans & 0x8000) != 0;
    let normalized = ((angle % 360.0) + 360.0) % 360.0;
    let rot = if (normalized - 0.0).abs() < 1e-9 || (normalized - 360.0).abs() < 1e-9 {
        Rot4::R0
    } else if (normalized - 90.0).abs() < 1e-9 {
        Rot4::R90
    } else if (normalized - 180.0).abs() < 1e-9 {
        Rot4::R180
    } else if (normalized - 270.0).abs() < 1e-9 {
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

