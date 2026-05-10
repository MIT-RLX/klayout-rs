//! Hierarchy editing primitives.
//!
//! All operations return new `CellBuilder`s — frozen cells in the
//! `Library` are never mutated. The caller chooses whether to insert the
//! result into the same library, a new library, or just inspect it.
//!
//! Available operations:
//! * [`flatten`] — produce a single cell containing all shapes from the
//!   instance subtree, transforms composed.
//! * [`remap_layers`] — return a new `CellBuilder` with shapes remapped to
//!   different layers per a `LayerMap`.
//! * [`replace_instances`] — rewrite instances of one cell to another by
//!   a user-supplied predicate.

use crate::{
    Bbox, CellBuilder, CellId, CellName, Instance, LayerIndex, Library, Polygon, Rect, Repetition,
    Shape, Trans, Vec2,
};
use std::collections::HashMap;

/// Flatten a cell's instance hierarchy `max_depth` levels deep into a
/// single cell. `max_depth = 0` is a no-op (no children flattened);
/// `max_depth = usize::MAX` flattens everything.
pub fn flatten(
    lib: &Library,
    cell: CellId,
    max_depth: usize,
    new_name: impl Into<CellName>,
) -> CellBuilder {
    let mut out = CellBuilder::new(new_name);
    walk(lib, cell, Trans::IDENTITY, max_depth, &mut out);
    out
}

fn walk(
    lib: &Library,
    cell: CellId,
    trans: Trans,
    depth_remaining: usize,
    out: &mut CellBuilder,
) {
    let c = lib.get(cell);
    // Copy shapes with the composed trans applied.
    for layer in c.layers() {
        for shape in c.shapes_on(layer) {
            let s = shape.transform(trans);
            out.add_shape(layer, s);
        }
    }
    if depth_remaining == 0 {
        // Keep instances as-is (with composed trans).
        for inst in c.instances() {
            let mut new_inst = inst.clone();
            new_inst.trans = trans.compose(inst.trans);
            out.add_instance(new_inst);
        }
        return;
    }
    for inst in c.instances() {
        for placement in expand_repetition(inst) {
            let composed = trans.compose(placement);
            walk(lib, inst.cell, composed, depth_remaining - 1, out);
        }
    }
}

fn expand_repetition(inst: &Instance) -> Vec<Trans> {
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
                    let extra = Trans::translate(Vec2::new(
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
                let extra = Trans::translate(*o);
                out.push(extra.compose(inst.trans));
            }
            out
        }
    }
}

/// A layer remapping table — `from` → `to` rewrites every shape on
/// `from` to land on `to`. Layers not in the map pass through unchanged.
#[derive(Default, Clone, Debug)]
pub struct LayerMap {
    inner: HashMap<LayerIndex, LayerIndex>,
}

impl LayerMap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn map(mut self, from: LayerIndex, to: LayerIndex) -> Self {
        self.inner.insert(from, to);
        self
    }

    pub fn translate(&self, layer: LayerIndex) -> LayerIndex {
        self.inner.get(&layer).copied().unwrap_or(layer)
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Rebuild `cell` with all shapes' layers translated through `map`.
/// Instances are preserved unchanged (the caller can also remap layers
/// on each child by walking the hierarchy themselves).
pub fn remap_layers(lib: &Library, cell: CellId, map: &LayerMap) -> CellBuilder {
    let c = lib.get(cell);
    let mut out = CellBuilder::new(c.name().clone());
    for layer in c.layers() {
        let dst = map.translate(layer);
        for shape in c.shapes_on(layer) {
            out.add_shape(dst, shape.clone());
        }
    }
    for inst in c.instances() {
        out.add_instance(inst.clone());
    }
    out
}

/// Clip a cell to `bbox` — return a `CellBuilder` containing only shapes
/// whose bbox intersects `bbox`. Boxes are clipped to `bbox`; polygons
/// and paths whose bbox extends past it are kept whole (proper polygon
/// clipping needs `klayout-geom`'s boolean ops — call those if exact
/// edge clipping matters). Instances are kept if their placement bbox
/// (`trans.disp`) lies inside `bbox` and dropped otherwise.
pub fn clip_cell(
    lib: &Library,
    cell: CellId,
    bbox: Bbox,
    new_name: impl Into<CellName>,
) -> CellBuilder {
    let c = lib.get(cell);
    let mut out = CellBuilder::new(new_name);
    if bbox.is_empty() {
        return out;
    }
    for layer in c.layers() {
        for shape in c.shapes_on(layer) {
            let sbb = shape.bbox();
            if !sbb.intersects(&bbox) {
                continue;
            }
            match shape {
                Shape::Box(r) => {
                    let clipped = bbox_intersection(r.bbox, bbox);
                    if !clipped.is_empty() {
                        out.add_shape(layer, Rect::new(clipped));
                    }
                }
                Shape::Polygon(p) => {
                    // Approximate: keep the polygon as-is. Exact clipping
                    // would use `klayout_geom::intersection`.
                    out.add_shape(layer, p.clone());
                }
                Shape::Path(p) => out.add_shape(layer, p.clone()),
                Shape::Text(t) => {
                    if bbox.contains(t.anchor) {
                        out.add_shape(layer, t.clone());
                    }
                }
            }
        }
    }
    for inst in c.instances() {
        // Approximate inclusion: keep instance if its anchor (trans.disp)
        // is inside the clip bbox. This is a coarse filter; users who
        // need precise instance clipping should flatten first.
        let anchor = klayout_core_alias::Point::new(inst.trans.disp.x, inst.trans.disp.y);
        if bbox.contains(anchor) {
            out.add_instance(inst.clone());
        }
    }
    out
}

mod klayout_core_alias {
    pub use crate::Point;
}

fn bbox_intersection(a: Bbox, b: Bbox) -> Bbox {
    if a.is_empty() || b.is_empty() {
        return Bbox::EMPTY;
    }
    let min = klayout_core_alias::Point::new(a.min.x.max(b.min.x), a.min.y.max(b.min.y));
    let max = klayout_core_alias::Point::new(a.max.x.min(b.max.x), a.max.y.min(b.max.y));
    if min.x > max.x || min.y > max.y {
        Bbox::EMPTY
    } else {
        Bbox::new(min, max)
    }
}

/// Bbox of the cell restricted to its top-level shapes (no descent into
/// instance children). Convenience for clip-at-top workflows.
pub fn local_bbox(lib: &Library, cell: CellId) -> Bbox {
    lib.get(cell).local_bbox()
}

#[allow(dead_code)]
fn _polygon_marker(_: &Polygon) {}

/// Rewrite instances of cells matching `predicate` to point at
/// `replacement`. The `predicate` is called with the original instance's
/// `CellId` and returns `true` to swap. Shapes are preserved.
pub fn replace_instances<F: Fn(CellId) -> bool>(
    lib: &Library,
    cell: CellId,
    predicate: F,
    replacement: CellId,
) -> CellBuilder {
    let c = lib.get(cell);
    let mut out = CellBuilder::new(c.name().clone());
    for layer in c.layers() {
        for shape in c.shapes_on(layer) {
            out.add_shape(layer, shape.clone());
        }
    }
    for inst in c.instances() {
        let mut new_inst = inst.clone();
        if predicate(inst.cell) {
            new_inst.cell = replacement;
        }
        out.add_instance(new_inst);
    }
    out
}

