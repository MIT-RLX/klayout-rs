//! Polygon vertex-reduction via Douglas-Peucker.
//!
//! Decimates a polygon's hull (and holes) by recursively dropping
//! vertices whose perpendicular distance to the chord between the
//! retained endpoints falls below `epsilon` (DBU). The classic
//! Douglas-Peucker algorithm — O(n log n) average, O(n²) worst.
//!
//! Useful for:
//! * Compacting GDS that comes from rasterised tools (every 1-DBU
//!   step encoded as its own vertex).
//! * Reducing manual-edited polygons before boolean ops.
//! * Stylising paths from non-manhattan routers.
//!
//! Convention: the algorithm preserves `epsilon = 0` exactly (no
//! decimation) and degenerates gracefully on hulls with `< 3`
//! vertices.

use klayout_core::{Point, Polygon};

/// Simplify a polygon's hull and holes with Douglas-Peucker. Returns
/// a new polygon; original is untouched.
pub fn simplify(polygon: &Polygon, epsilon: i64) -> Polygon {
    if epsilon <= 0 || polygon.hull.len() < 4 {
        return polygon.clone();
    }
    let new_hull = simplify_ring(&polygon.hull, epsilon);
    let mut p = Polygon::from_hull(new_hull);
    for hole in &polygon.holes {
        let new_hole = simplify_ring(hole, epsilon);
        if new_hole.len() >= 3 {
            p.add_hole(new_hole);
        }
    }
    p
}

/// Simplify a closed polyline (closing edge implicit between last and
/// first). Returns a vertex list with the same closure semantics.
pub fn simplify_ring(pts: &[Point], epsilon: i64) -> Vec<Point> {
    let n = pts.len();
    if n < 4 {
        return pts.to_vec();
    }
    // Pick the two extreme vertices on the ring to anchor the
    // recursion. For closed loops we use the first and the farthest
    // vertex from it, then process the ring as two open chains.
    let anchor = 0;
    let mut farthest = 1;
    let mut max_dist2: i128 = 0;
    for i in 1..n {
        let d = dist2(pts[anchor], pts[i]);
        if d > max_dist2 {
            max_dist2 = d;
            farthest = i;
        }
    }
    let mut out: Vec<Point> = Vec::new();
    let chain_a: Vec<Point> = pts[anchor..=farthest].to_vec();
    let mut chain_b: Vec<Point> = pts[farthest..].to_vec();
    chain_b.push(pts[anchor]);
    let simp_a = simplify_open(&chain_a, epsilon);
    let simp_b = simplify_open(&chain_b, epsilon);
    out.extend_from_slice(&simp_a);
    if simp_b.len() >= 2 {
        out.extend_from_slice(&simp_b[1..simp_b.len() - 1]);
    }
    if out.len() >= 2 && out.first() == out.last() {
        out.pop();
    }
    out
}

/// Simplify an open polyline (first and last vertices retained).
pub fn simplify_open(pts: &[Point], epsilon: i64) -> Vec<Point> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    // Length ≥ 3 here (early return above), so first/last indices exist.
    let mut keep = vec![false; pts.len()];
    let last_idx = pts.len() - 1;
    keep[0] = true;
    keep[last_idx] = true;
    rdp(pts, 0, last_idx, epsilon, &mut keep);
    pts.iter()
        .zip(keep.iter())
        .filter_map(|(p, k)| if *k { Some(*p) } else { None })
        .collect()
}

fn rdp(pts: &[Point], lo: usize, hi: usize, epsilon: i64, keep: &mut [bool]) {
    if hi <= lo + 1 {
        return;
    }
    let a = pts[lo];
    let b = pts[hi];
    let mut max_d2: i128 = 0;
    let mut max_idx = lo;
    let eps2 = (epsilon as i128) * (epsilon as i128);
    for (i, p) in pts.iter().enumerate().take(hi).skip(lo + 1) {
        let d2 = perp_dist2(*p, a, b);
        if d2 > max_d2 {
            max_d2 = d2;
            max_idx = i;
        }
    }
    if max_d2 > eps2 {
        keep[max_idx] = true;
        rdp(pts, lo, max_idx, epsilon, keep);
        rdp(pts, max_idx, hi, epsilon, keep);
    }
}

/// Squared perpendicular distance (× area_factor²) from `p` to the
/// line through `a-b`. Returned as i128 to avoid overflow on large
/// DBU coordinates and to keep the comparison exact.
fn perp_dist2(p: Point, a: Point, b: Point) -> i128 {
    let dx = (b.x - a.x) as i128;
    let dy = (b.y - a.y) as i128;
    let len2 = dx * dx + dy * dy;
    if len2 == 0 {
        return dist2(p, a);
    }
    // Cross product gives 2 × signed triangle area.
    let cross = dx * (a.y - p.y) as i128 - dy * (a.x - p.x) as i128;
    (cross * cross) / len2
}

fn dist2(a: Point, b: Point) -> i128 {
    let dx = (b.x - a.x) as i128;
    let dy = (b.y - a.y) as i128;
    dx * dx + dy * dy
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::Polygon;

    fn pt(x: i64, y: i64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn already_simple_polygon_unchanged() {
        // Square with no extra vertices.
        let pts = vec![pt(0, 0), pt(0, 10), pt(10, 10), pt(10, 0)];
        let s = simplify_ring(&pts, 1);
        assert_eq!(s.len(), 4);
    }

    #[test]
    fn collinear_vertices_dropped() {
        // Square with extra mid-edge vertices.
        let pts = vec![
            pt(0, 0),
            pt(0, 5),
            pt(0, 10),
            pt(5, 10),
            pt(10, 10),
            pt(10, 5),
            pt(10, 0),
            pt(5, 0),
        ];
        let s = simplify_ring(&pts, 1);
        assert_eq!(s.len(), 4, "collinear vertices should drop");
    }

    #[test]
    fn near_collinear_dropped_within_epsilon() {
        // Tiny y-jog (1 DBU) that should drop at epsilon ≥ 1.
        let pts = vec![
            pt(0, 0),
            pt(50, 0),
            pt(60, 1),
            pt(100, 0),
            pt(100, 50),
            pt(0, 50),
        ];
        let s = simplify_ring(&pts, 2);
        assert!(s.len() <= 5, "1-DBU jog should drop at eps=2");
    }

    #[test]
    fn large_features_preserved() {
        let pts = vec![
            pt(0, 0),
            pt(50, 0),
            pt(50, 30),  // big jog
            pt(100, 0),
            pt(100, 50),
            pt(0, 50),
        ];
        let s = simplify_ring(&pts, 2);
        // The (50, 30) vertex should be preserved.
        assert!(s.iter().any(|p| p.y == 30));
    }

    #[test]
    fn polygon_simplify_preserves_holes() {
        let hull = vec![pt(0, 0), pt(0, 100), pt(100, 100), pt(100, 0)];
        let mut p = Polygon::from_hull(hull);
        // Hole with extra collinear vertices.
        let hole = vec![
            pt(40, 40),
            pt(40, 50),
            pt(40, 60),
            pt(60, 60),
            pt(60, 50),
            pt(60, 40),
        ];
        p.add_hole(hole);
        let s = simplify(&p, 1);
        assert_eq!(s.holes.len(), 1);
        assert!(s.holes[0].len() <= 4);
    }

    #[test]
    fn epsilon_zero_no_change() {
        let pts = vec![pt(0, 0), pt(1, 0), pt(2, 0), pt(0, 5)];
        let p = Polygon::from_hull(pts);
        let s = simplify(&p, 0);
        assert_eq!(s.hull.len(), p.hull.len());
    }
}
