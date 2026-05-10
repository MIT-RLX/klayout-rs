//! DRC primitives consuming `Edges` (klayout-geom) directly.
//!
//! KLayout's deck-style ergonomics revolve around chained edge
//! operators: `metal1.edges.with_angle(0).space_check(other.edges, 100)`.
//! The polygon-input rules in `rules.rs` are convenient for whole-layer
//! checks; these edge-input rules let callers filter the edge set
//! first (by angle, length, position) and then run the same edge-pair
//! geometry on the survivors.
//!
//! All three primitives reuse the general-angle pair kernel from
//! `edge_general.rs`, so they handle axis-aligned and arbitrary-angle
//! input uniformly.

use crate::edge_general::{general_pair, DirEdge};
use klayout_core::Polygon;
use klayout_geom::{merge, Edge, Edges, Region};
use rayon::prelude::*;

fn to_dir(e: Edge) -> DirEdge {
    DirEdge { a: e.a, b: e.b }
}

/// Width-check semantics on an explicit edge set: each pair of edges
/// in `edges` is checked as a same-polygon facing pair (interior
/// normals point at each other) with perpendicular distance < `min`.
pub fn width_edges(edges: &Edges, min: i64) -> Region {
    pair_check_edges(edges, edges, min, true)
}

/// Space-check on a single edge set — pairs of edges with exterior
/// normals facing each other through a gap < `min`.
pub fn space_edges(edges: &Edges, min: i64) -> Region {
    pair_check_edges(edges, edges, min, false)
}

/// Two-input separation check across two edge collections.
pub fn separation_edges(a: &Edges, b: &Edges, min: i64) -> Region {
    pair_check_edges_cross(a, b, min)
}

fn pair_check_edges(a: &Edges, b: &Edges, min: i64, is_width: bool) -> Region {
    if min <= 0 || a.is_empty() || b.is_empty() {
        return Region::empty();
    }
    let dir_a: Vec<DirEdge> = a.edges().iter().copied().map(to_dir).collect();
    let dir_b: Vec<DirEdge> = b.edges().iter().copied().map(to_dir).collect();
    let same = std::ptr::eq(a, b) || a.edges().len() == b.edges().len();
    let polys: Vec<Polygon> = (0..dir_a.len())
        .into_par_iter()
        .flat_map_iter(|i| {
            let mut local = Vec::new();
            let start_j = if same { i + 1 } else { 0 };
            for eb in dir_b.iter().skip(start_j) {
                if let Some(p) = general_pair(&dir_a[i], eb, min, is_width) {
                    local.push(p);
                }
            }
            local
        })
        .collect();
    polygons_to_region(polys)
}

fn pair_check_edges_cross(a: &Edges, b: &Edges, min: i64) -> Region {
    if min <= 0 || a.is_empty() || b.is_empty() {
        return Region::empty();
    }
    let dir_a: Vec<DirEdge> = a.edges().iter().copied().map(to_dir).collect();
    let dir_b: Vec<DirEdge> = b.edges().iter().copied().map(to_dir).collect();
    let polys: Vec<Polygon> = (0..dir_a.len())
        .into_par_iter()
        .flat_map_iter(|i| {
            let mut local = Vec::new();
            for eb in &dir_b {
                if let Some(p) = general_pair(&dir_a[i], eb, min, false) {
                    local.push(p);
                }
            }
            local
        })
        .collect();
    polygons_to_region(polys)
}

fn polygons_to_region(polys: Vec<Polygon>) -> Region {
    if polys.is_empty() {
        return Region::empty();
    }
    merge(&Region::from_polygons(polys))
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::{Bbox, Point, Polygon};

    fn rect_edges(b: Bbox) -> Edges {
        Edges::from_polygon(&Polygon::rect(b))
    }

    #[test]
    fn width_edges_matches_polygon_width_on_thin_wire() {
        let r = Region::from_polygons([Polygon::rect(Bbox::new(
            Point::new(0, 0),
            Point::new(100, 5),
        ))]);
        let e = Edges::from_region(&r);
        let v = width_edges(&e, 10);
        assert!(!v.is_empty());
    }

    #[test]
    fn width_edges_only_on_filtered_subset() {
        // 5-tall thin wire — all edges are AA.
        // Filter to only horizontal edges → width should still trigger
        // (top and bottom both horizontal).
        let r = Region::from_polygons([Polygon::rect(Bbox::new(
            Point::new(0, 0),
            Point::new(100, 5),
        ))]);
        let horizontals = Edges::from_region(&r).with_angle(0.0, 1.0);
        assert_eq!(horizontals.len(), 2);
        let v = width_edges(&horizontals, 10);
        assert!(!v.is_empty());
    }

    #[test]
    fn space_edges_finds_gap_violation() {
        let r = Region::from_polygons([
            Polygon::rect(Bbox::new(Point::new(0, 0), Point::new(10, 10))),
            Polygon::rect(Bbox::new(Point::new(15, 0), Point::new(25, 10))), // 5 apart
        ]);
        let e = Edges::from_region(&r);
        let v = space_edges(&e, 10);
        assert!(!v.is_empty());
    }

    #[test]
    fn separation_edges_across_two_sets() {
        let r_a = Region::from_polygons([Polygon::rect(Bbox::new(
            Point::new(0, 0),
            Point::new(10, 10),
        ))]);
        let r_b = Region::from_polygons([Polygon::rect(Bbox::new(
            Point::new(15, 0),
            Point::new(25, 10),
        ))]);
        let v = separation_edges(
            &Edges::from_region(&r_a),
            &Edges::from_region(&r_b),
            10,
        );
        assert!(!v.is_empty());
    }

    #[test]
    fn chained_filter_then_check() {
        // m1 wires running vertically should pass a horizontal width
        // check (no horizontal opposite-facing pair in the filter).
        let r = Region::from_polygons([Polygon::rect(Bbox::new(
            Point::new(0, 0),
            Point::new(5, 100),
        ))]);
        let horiz_only = Edges::from_region(&r)
            .with_angle(0.0, 1.0)
            .length_at_least(1);
        assert_eq!(horiz_only.len(), 2); // top + bottom
        // Two horizontal edges 100 apart; min=10 → no violation.
        let v = width_edges(&horiz_only, 10);
        assert!(v.is_empty());
        // But min=200 catches the 100-wide width.
        let v2 = width_edges(&horiz_only, 200);
        assert!(!v2.is_empty());
    }

    #[test]
    fn empty_inputs_yield_empty_output() {
        assert!(width_edges(&Edges::empty(), 10).is_empty());
        assert!(space_edges(&Edges::empty(), 10).is_empty());
        let _ = rect_edges; // keep helper
    }
}
