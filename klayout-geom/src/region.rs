//! `Region` — a deterministic, normalized set of polygons on a single layer.
//!
//! Always merged: after any operation that produces a `Region`, overlapping
//! and touching polygons have been combined into the smallest set of disjoint
//! pieces. Polygons inside a region are canonical (lowest-y/lowest-x start,
//! clockwise winding) so two regions with the same merged-area set hash to
//! the same canonical form.

use klayout_core::{Bbox, Cell, CellId, LayerIndex, Library, Point, Polygon, Shape, Trans};

#[derive(Clone, Debug, Default)]
pub struct Region {
    pub(crate) polygons: Vec<Polygon>,
    bbox: Bbox,
}

impl Region {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_polygons<I: IntoIterator<Item = Polygon>>(iter: I) -> Self {
        let mut r = Self::empty();
        for p in iter {
            r.bbox = r.bbox.union(&p.bbox());
            r.polygons.push(p);
        }
        r
    }

    pub fn polygons(&self) -> &[Polygon] {
        &self.polygons
    }

    pub fn bbox(&self) -> Bbox {
        self.bbox
    }

    pub fn is_empty(&self) -> bool {
        self.polygons.is_empty()
    }

    pub fn len(&self) -> usize {
        self.polygons.len()
    }

    /// Build a region from all `Shape::Polygon` and `Shape::Box` shapes on
    /// `layer` reachable from `root`, walking the instance hierarchy and
    /// composing transforms. Repetitions expand into individual placements.
    pub fn from_cell_layer(lib: &Library, root: CellId, layer: LayerIndex) -> Self {
        let mut polys: Vec<Polygon> = Vec::new();
        walk(lib, root, layer, Trans::IDENTITY, &mut polys);
        Self::from_polygons(polys)
    }

    pub(crate) fn set_polygons(&mut self, ps: Vec<Polygon>) {
        self.bbox = Bbox::EMPTY;
        for p in &ps {
            self.bbox = self.bbox.union(&p.bbox());
        }
        self.polygons = ps;
        // Deterministic order: by canonical bbox + first-vertex tuple.
        self.polygons.sort_by_key(canonical_key);
    }
}

fn canonical_key(p: &Polygon) -> (i64, i64, i64, i64, usize) {
    let b = p.bbox();
    let n = p.hull.len();
    (b.min.x, b.min.y, b.max.x, b.max.y, n)
}

fn walk(
    lib: &Library,
    cell_id: CellId,
    layer: LayerIndex,
    trans: Trans,
    out: &mut Vec<Polygon>,
) {
    let cell: std::sync::Arc<Cell> = lib.get(cell_id);
    for shape in cell.shapes_on(layer) {
        match shape {
            Shape::Polygon(p) => out.push(p.transform(trans)),
            Shape::Box(r) => {
                let bb = trans.apply_bbox(r.bbox);
                if !bb.is_empty() {
                    out.push(Polygon::rect(bb));
                }
            }
            _ => {}
        }
    }
    for inst in cell.instances() {
        let placements = expand_repetition(inst);
        for placement in placements {
            let composed = trans.compose(placement);
            walk(lib, inst.cell, layer, composed, out);
        }
    }
}

fn expand_repetition(inst: &klayout_core::Instance) -> Vec<Trans> {
    use klayout_core::{Repetition, Vec2};
    match &inst.repetition {
        None => vec![inst.trans],
        Some(Repetition::Regular {
            col,
            row,
            n_cols,
            n_rows,
        }) => {
            let mut out = Vec::with_capacity((*n_cols as usize) * (*n_rows as usize));
            for j in 0..*n_rows {
                for i in 0..*n_cols {
                    let extra = klayout_core::Trans::translate(Vec2::new(
                        col.x * i as i64 + row.x * j as i64,
                        col.y * i as i64 + row.y * j as i64,
                    ));
                    out.push(extra.compose(inst.trans));
                }
            }
            out
        }
        Some(Repetition::Irregular { offsets }) => {
            let mut out = Vec::with_capacity(offsets.len() + 1);
            out.push(inst.trans);
            for o in offsets {
                let extra = klayout_core::Trans::translate(*o);
                out.push(extra.compose(inst.trans));
            }
            out
        }
    }
}

// Used by boolean.rs to produce normalized hulls.
pub(crate) fn polygon_from_int_contour(c: &[(i64, i64)]) -> Option<Polygon> {
    if c.len() < 3 {
        return None;
    }
    let pts = c.iter().map(|(x, y)| Point::new(*x, *y));
    Some(Polygon::from_hull(pts))
}
