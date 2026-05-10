//! Arbitrary-angle edge-pair DRC kernel.
//!
//! The axis-aligned kernel in `edge.rs` assumes every polygon edge runs
//! along an axis. That's true for the bulk of real layouts but fails on
//! 45°-routed analog blocks, RF tapered transmission lines, and
//! rotated-instance bumps. This module generalizes the same edge-pair
//! semantics to arbitrary angles using vector math:
//!
//! * Two edges are a candidate pair if they're parallel (anti-parallel
//!   for opposite traversal directions) — `cross(d1, d2) ≈ 0`.
//! * Their perpendicular distance must be `< min` and `> 0`.
//! * Their interior-normals must point at each other (width) or
//!   exterior-normals must point at each other (space).
//! * Their projections on the common direction must overlap.
//!
//! Violations emit as a four-vertex parallelogram bounded by the
//! projection-overlap span on each edge. For axis-aligned input this
//! coincides exactly with the `edge.rs` output (verified by parity
//! tests).
//!
//! Sub-DBU precision: vectors live in `f64` internally, with output
//! vertices rounded to `i64`. The polygon CW hull convention is
//! preserved — interior is on the right of each edge.

use klayout_core::{Point, Polygon};

#[derive(Copy, Clone, Debug)]
pub(crate) struct DirEdge {
    pub a: Point,
    pub b: Point,
}

impl DirEdge {
    fn dx(&self) -> f64 {
        (self.b.x - self.a.x) as f64
    }
    fn dy(&self) -> f64 {
        (self.b.y - self.a.y) as f64
    }
    fn length(&self) -> f64 {
        (self.dx() * self.dx() + self.dy() * self.dy()).sqrt()
    }
    fn unit(&self) -> (f64, f64) {
        let len = self.length();
        if len == 0.0 {
            return (0.0, 0.0);
        }
        (self.dx() / len, self.dy() / len)
    }
}

/// Emit one `DirEdge` per non-degenerate hull segment. Holes are
/// included on the assumption the polygon hull is CW (so hole edges
/// also walk with interior-on-the-right when holes are CCW).
pub(crate) fn polygon_dir_edges(p: &Polygon) -> Vec<DirEdge> {
    let mut out = Vec::new();
    for ring in std::iter::once(p.hull.as_slice()).chain(p.holes.iter().map(|h| h.as_slice())) {
        let n = ring.len();
        for i in 0..n {
            let a = ring[i];
            let b = ring[(i + 1) % n];
            if a != b {
                out.push(DirEdge { a, b });
            }
        }
    }
    out
}

/// General-angle pair check. Returns the violation polygon (a
/// parallelogram) if `e1` and `e2` form a valid pair under the chosen
/// `is_width` semantics.
///
/// `is_width` selects which side faces inward:
/// * `true` — interior normals must point at each other (e1 and e2
///   bound a region of *one* polygon's interior narrower than `min`).
/// * `false` — exterior normals must point at each other (e1 and e2
///   bound a gap between two polygons narrower than `min`).
pub(crate) fn general_pair(e1: &DirEdge, e2: &DirEdge, min: i64, is_width: bool) -> Option<Polygon> {
    let (ux1, uy1) = e1.unit();
    let (ux2, uy2) = e2.unit();
    if (ux1, uy1) == (0.0, 0.0) || (ux2, uy2) == (0.0, 0.0) {
        return None;
    }

    // Parallel (anti-parallel needed for facing pairs).
    let cross = ux1 * uy2 - uy1 * ux2;
    if cross.abs() > 1e-9 {
        return None;
    }
    let dot = ux1 * ux2 + uy1 * uy2;
    if dot >= 0.0 {
        // Same direction — they're collinear or parallel-not-facing.
        // Both width and space pairs require anti-parallel traversal.
        return None;
    }

    // Right-side perpendicular of e1 (interior normal for CW hull).
    // perp_right(d) = (dy, -dx).
    let nx = uy1;
    let ny = -ux1;

    // Vector from a1 to a2; project on n. Positive value means a2 is
    // on e1's interior side.
    let dx = (e2.a.x - e1.a.x) as f64;
    let dy = (e2.a.y - e1.a.y) as f64;
    let signed_dist = dx * nx + dy * ny;
    let abs_dist = signed_dist.abs();
    if abs_dist <= 0.0 || abs_dist >= min as f64 {
        return None;
    }

    // Facing check:
    // * is_width: e2 is on e1's interior side → signed_dist > 0.
    // * is_space: e2 is on e1's exterior side → signed_dist < 0.
    if is_width {
        if signed_dist <= 0.0 {
            return None;
        }
    } else if signed_dist >= 0.0 {
        return None;
    }

    // Projection-overlap on e1's tangent direction.
    let tx = ux1;
    let ty = uy1;
    let t_a1: f64 = 0.0;
    let t_b1: f64 = (e1.dx() * tx + e1.dy() * ty).max(0.0);
    let t_a2 = (e2.a.x - e1.a.x) as f64 * tx + (e2.a.y - e1.a.y) as f64 * ty;
    let t_b2 = (e2.b.x - e1.a.x) as f64 * tx + (e2.b.y - e1.a.y) as f64 * ty;
    // e2 walks the opposite direction along the tangent (anti-parallel),
    // so its t-range is [min(t_a2, t_b2), max(t_a2, t_b2)].
    let lo2 = t_a2.min(t_b2);
    let hi2 = t_a2.max(t_b2);

    let t_lo = t_a1.max(lo2);
    let t_hi = t_b1.min(hi2);
    if t_hi <= t_lo {
        return None;
    }

    // Build the four vertices. For e1: at t_lo and t_hi along its
    // direction. For e2: at the same t (in e1's frame), projected onto
    // e2's line by adding the perpendicular offset.
    let p_e1_lo = (
        e1.a.x as f64 + t_lo * tx,
        e1.a.y as f64 + t_lo * ty,
    );
    let p_e1_hi = (
        e1.a.x as f64 + t_hi * tx,
        e1.a.y as f64 + t_hi * ty,
    );
    // e2 is offset from e1 by (signed_dist * (nx, ny)).
    let p_e2_lo = (
        p_e1_lo.0 + signed_dist * nx,
        p_e1_lo.1 + signed_dist * ny,
    );
    let p_e2_hi = (
        p_e1_hi.0 + signed_dist * nx,
        p_e1_hi.1 + signed_dist * ny,
    );

    // CW hull: e1 segment, then to e2's hi corner, e2 segment back, to e1's lo corner.
    Some(Polygon::from_hull(vec![
        round_pt(p_e1_lo),
        round_pt(p_e1_hi),
        round_pt(p_e2_hi),
        round_pt(p_e2_lo),
    ]))
}

fn round_pt(p: (f64, f64)) -> Point {
    Point::new(p.0.round() as i64, p.1.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::{Bbox, Point};

    #[test]
    fn parallel_horizontals_yield_pair() {
        // CW hull edges of a 20×5 rect: bottom edge runs west, top edge
        // east. Both interiors face inward — width pair, distance 5,
        // min=10 → violation.
        let bottom = DirEdge {
            a: Point::new(20, 0),
            b: Point::new(0, 0),
        };
        let top = DirEdge {
            a: Point::new(0, 5),
            b: Point::new(20, 5),
        };
        let v = general_pair(&bottom, &top, 10, true).unwrap();
        assert_eq!(v.hull.len(), 4);
    }

    #[test]
    fn general_kernel_matches_aa_for_horizontals() {
        let bottom = DirEdge {
            a: Point::new(10, 0),
            b: Point::new(0, 0),
        };
        let top = DirEdge {
            a: Point::new(0, 5),
            b: Point::new(10, 5),
        };
        let v = general_pair(&bottom, &top, 10, true).unwrap();
        // Should produce a 10x5 rectangle.
        let bb = v.bbox();
        assert_eq!(bb, Bbox::new(Point::new(0, 0), Point::new(10, 5)));
    }

    #[test]
    fn diagonal_pair_under_min_distance() {
        // Two parallel 45° edges, perpendicular distance ≈ 4 (< min=10).
        // e1 (0,0)→(10,10): direction (1,1)/√2; interior-right normal = (1,-1)/√2.
        // Place e2 at e1 offset by 4 along the interior normal, walking
        // anti-parallel.
        let off = 4.0 / 2f64.sqrt();
        let e1 = DirEdge {
            a: Point::new(0, 0),
            b: Point::new(10, 10),
        };
        let e2_a = (10.0 + off, 10.0 - off);
        let e2_b = (off, -off);
        let e2 = DirEdge {
            a: Point::new(e2_a.0.round() as i64, e2_a.1.round() as i64),
            b: Point::new(e2_b.0.round() as i64, e2_b.1.round() as i64),
        };
        let v = general_pair(&e1, &e2, 10, true);
        assert!(v.is_some(), "expected width violation on parallel 45° pair");
    }

    #[test]
    fn same_direction_pair_rejected() {
        let e1 = DirEdge {
            a: Point::new(0, 0),
            b: Point::new(10, 0),
        };
        let e2 = DirEdge {
            a: Point::new(0, 5),
            b: Point::new(10, 5),
        };
        // Same direction (both left-to-right) — not a facing pair.
        assert!(general_pair(&e1, &e2, 10, true).is_none());
    }
}
