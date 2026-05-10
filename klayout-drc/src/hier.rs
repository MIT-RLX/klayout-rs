//! Hierarchical DRC.
//!
//! Flat DRC flattens the layer first, then runs rules. That's correct
//! but pays the rule cost for every instance of every cell — a 100-row
//! standard-cell array runs the rule 100× even though every row sees
//! the same internal geometry. KLayout's hierarchical DRC fixes this:
//! each unique cell is checked once, and only the inter-instance
//! "stitch" zones (within `halo` of an instance boundary) are checked
//! globally.
//!
//! v1 algorithm:
//! 1. Walk the hierarchy, hashing cells by `content_hash`. For each
//!    unique cell, run the rule on its *own* shapes (no instances).
//!    Cache by hash.
//! 2. Emit cached violations transformed under each instance.
//! 3. Build a "stitch region" — geometry within `halo` of any instance
//!    bbox at every level — and run the rule on that. The stitch region
//!    is small relative to the full layout when cells are well-pitched.
//! 4. Merge phase-2 + phase-3 results.
//!
//! Caveat: the stitch region is currently the *flattened* layer clipped
//! to the union of all instance-boundary halos. For deeply nested
//! hierarchies this is conservative — the same geometry may be checked
//! at multiple levels. KLayout has a more sophisticated stitching pass
//! that runs per cell on neighborhood-only geometry; that's a v2.

use klayout_core::{Bbox, CellId, ContentHash, LayerIndex, Library, Point, Polygon, Trans};
use klayout_geom::{intersection, merge, Region};
use std::collections::HashMap;

/// Run a unary DRC rule hierarchically. Returns the union of
/// flat-equivalent violations across the entire layout.
pub fn hierarchical_check<F>(
    lib: &Library,
    top: CellId,
    layer: LayerIndex,
    halo: i64,
    rule: F,
) -> Region
where
    F: Fn(&Region) -> Region + Sync,
{
    let mut cache: HashMap<ContentHash, Region> = HashMap::new();
    let mut emitted: Vec<Polygon> = Vec::new();

    walk(
        lib, top, Trans::IDENTITY, layer, &rule, &mut cache, &mut emitted,
    );

    if halo > 0 {
        let stitch = stitch_region(lib, top, layer, halo);
        let stitch_v = rule(&stitch);
        emitted.extend(stitch_v.polygons().iter().cloned());
    }

    if emitted.is_empty() {
        return Region::empty();
    }
    merge(&Region::from_polygons(emitted))
}

fn walk<F>(
    lib: &Library,
    cell_id: CellId,
    accum: Trans,
    layer: LayerIndex,
    rule: &F,
    cache: &mut HashMap<ContentHash, Region>,
    emitted: &mut Vec<Polygon>,
) where
    F: Fn(&Region) -> Region + Sync,
{
    let cell = lib.get(cell_id);
    let hash = cell.content_hash();

    let local_violations = cache
        .entry(hash)
        .or_insert_with(|| rule(&local_region(lib, cell_id, layer)))
        .clone();

    for p in local_violations.polygons() {
        emitted.push(p.transform(accum));
    }

    for inst in cell.instances() {
        for placement in expand_inst(inst) {
            let composed = accum.compose(placement);
            walk(lib, inst.cell, composed, layer, rule, cache, emitted);
        }
    }
}

/// Region of the cell's *own* shapes on `layer` (no instance descent).
fn local_region(lib: &Library, cell_id: CellId, layer: LayerIndex) -> Region {
    use klayout_core::Shape;
    let cell = lib.get(cell_id);
    let mut polys: Vec<Polygon> = Vec::new();
    for shape in cell.shapes_on(layer) {
        match shape {
            Shape::Polygon(p) => polys.push(p.clone()),
            Shape::Box(r) => polys.push(Polygon::rect(r.bbox)),
            _ => {}
        }
    }
    let _ = lib; // kept for future per-library lookups
    if polys.is_empty() {
        return Region::empty();
    }
    Region::from_polygons(polys)
}

fn expand_inst(inst: &klayout_core::Instance) -> Vec<Trans> {
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
            let mut out = Vec::with_capacity(offsets.len());
            for o in offsets {
                let extra = Trans::translate(*o);
                out.push(extra.compose(inst.trans));
            }
            out
        }
    }
}

/// Build a region containing all geometry on `layer` within `halo` of
/// any instance bbox boundary (at any nesting depth). Inter-instance
/// violations are detectable from this region alone.
fn stitch_region(lib: &Library, top: CellId, layer: LayerIndex, halo: i64) -> Region {
    let flat = Region::from_cell_layer(lib, top, layer);
    let mut zones: Vec<Polygon> = Vec::new();
    collect_stitch_zones(lib, top, Trans::IDENTITY, halo, &mut zones);
    if zones.is_empty() {
        return Region::empty();
    }
    let zones_region = merge(&Region::from_polygons(zones));
    intersection(&flat, &zones_region)
}

fn collect_stitch_zones(
    lib: &Library,
    cell_id: CellId,
    accum: Trans,
    halo: i64,
    out: &mut Vec<Polygon>,
) {
    let cell = lib.get(cell_id);
    for inst in cell.instances() {
        for placement in expand_inst(inst) {
            let composed = accum.compose(placement);
            // Bbox of the instance subtree, transformed.
            let child = lib.get(inst.cell);
            let child_bbox = child.full_bbox(lib);
            if child_bbox.is_empty() {
                continue;
            }
            let placed = composed.apply_bbox(child_bbox);
            // Halo around the instance boundary: a thick frame.
            let frame = boundary_frame(placed, halo);
            out.extend(frame);
            collect_stitch_zones(lib, inst.cell, composed, halo, out);
        }
    }
}

/// Return four rectangles forming a frame of width `halo` around `b`.
/// Each rectangle is `halo` wide and represents one side's halo zone.
fn boundary_frame(b: Bbox, halo: i64) -> Vec<Polygon> {
    if b.is_empty() || halo <= 0 {
        return Vec::new();
    }
    vec![
        // Top frame
        Polygon::rect(Bbox::new(
            Point::new(b.min.x - halo, b.max.y - halo),
            Point::new(b.max.x + halo, b.max.y + halo),
        )),
        // Bottom frame
        Polygon::rect(Bbox::new(
            Point::new(b.min.x - halo, b.min.y - halo),
            Point::new(b.max.x + halo, b.min.y + halo),
        )),
        // Left frame
        Polygon::rect(Bbox::new(
            Point::new(b.min.x - halo, b.min.y - halo),
            Point::new(b.min.x + halo, b.max.y + halo),
        )),
        // Right frame
        Polygon::rect(Bbox::new(
            Point::new(b.max.x - halo, b.min.y - halo),
            Point::new(b.max.x + halo, b.max.y + halo),
        )),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{space, width};
    use klayout_core::{
        Bbox as Bb, CellBuilder, Instance, LayerInfo, Library, Point as P, Rect, Trans, Vec2,
    };

    fn b(x0: i64, y0: i64, x1: i64, y1: i64) -> Bb {
        Bb::new(P::new(x0, y0), P::new(x1, y1))
    }

    #[test]
    fn flat_layout_matches_flat_drc() {
        let lib = Library::new("t", 1);
        let l = lib.layer(LayerInfo::gds(1, 0));
        let mut cb = CellBuilder::new("top");
        cb.add_shape(l, Rect::new(b(0, 0, 50, 5)));
        let id = lib.insert(cb);

        let h = hierarchical_check(&lib, id, l, 100, |r| width(r, 10));
        let flat = width(&Region::from_cell_layer(&lib, id, l), 10);
        assert_eq!(h.len(), flat.len());
    }

    #[test]
    fn cached_per_cell_check_finds_internal_violation() {
        let lib = Library::new("t", 1);
        let l = lib.layer(LayerInfo::gds(1, 0));

        // child has a thin shape that violates width.
        let mut cb = CellBuilder::new("child");
        cb.add_shape(l, Rect::new(b(0, 0, 50, 5))); // 5 units thin
        let child_id = lib.insert(cb);

        // parent places child twice.
        let mut pb = CellBuilder::new("top");
        pb.add_instance(Instance::new(child_id, Trans::IDENTITY));
        pb.add_instance(Instance::new(child_id, Trans::translate(Vec2::new(100, 0))));
        let top_id = lib.insert(pb);

        // width(10) on a 5-wide rect → both instances contribute.
        let h = hierarchical_check(&lib, top_id, l, 0, |r| width(r, 10));
        let flat = width(&Region::from_cell_layer(&lib, top_id, l), 10);
        assert_eq!(h.len(), flat.len());
        assert!(!h.is_empty());
    }

    #[test]
    fn stitch_region_catches_cross_instance_violation() {
        let lib = Library::new("t", 1);
        let l = lib.layer(LayerInfo::gds(1, 0));

        // child has one wide shape with no internal width violation.
        let mut cb = CellBuilder::new("child");
        cb.add_shape(l, Rect::new(b(0, 0, 50, 50)));
        let child_id = lib.insert(cb);

        // Place two instances 5 units apart — should violate space=10.
        let mut pb = CellBuilder::new("top");
        pb.add_instance(Instance::new(child_id, Trans::IDENTITY));
        pb.add_instance(Instance::new(
            child_id,
            Trans::translate(Vec2::new(55, 0)),
        ));
        let top_id = lib.insert(pb);

        // halo=15 covers the 5-unit gap.
        let h = hierarchical_check(&lib, top_id, l, 15, |r| space(r, 10));
        let flat = space(&Region::from_cell_layer(&lib, top_id, l), 10);
        // Both should detect the space violation.
        assert_eq!(h.is_empty(), flat.is_empty());
        assert!(!h.is_empty(), "stitch should catch cross-instance space");
    }

    #[test]
    fn cache_dedups_identical_cells() {
        // Synthesise a layout with 4 instances of the same cell. We can
        // only observe the cache effect through correctness — the
        // result must equal the flat result.
        let lib = Library::new("t", 1);
        let l = lib.layer(LayerInfo::gds(1, 0));
        let mut cb = CellBuilder::new("c");
        cb.add_shape(l, Rect::new(b(0, 0, 4, 4)));
        let cid = lib.insert(cb);
        let mut pb = CellBuilder::new("top");
        for i in 0..4 {
            pb.add_instance(Instance::new(cid, Trans::translate(Vec2::new(i * 100, 0))));
        }
        let top = lib.insert(pb);
        let h = hierarchical_check(&lib, top, l, 0, |r| width(r, 5));
        let flat = width(&Region::from_cell_layer(&lib, top, l), 5);
        assert_eq!(h.len(), flat.len());
    }
}
