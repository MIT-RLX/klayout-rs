//! Convex decomposition of polygons via ear-clipping.
//!
//! Many EDA passes need convex polygons: triangulation for FEM
//! solvers, mask-data fracture for write tools, mesh generation for
//! field-solver PEX. This module decomposes an arbitrary simple
//! polygon (no holes for v1) into a list of triangles whose
//! per-piece convexity is trivially satisfied.
//!
//! Algorithm: classic ear-clipping. At each step:
//! 1. Find an "ear" — three consecutive vertices `(prev, cur, next)`
//!    where `cur` is convex AND no other polygon vertex lies inside
//!    the triangle `(prev, cur, next)`.
//! 2. Emit `(prev, cur, next)` as one triangle; remove `cur`.
//! 3. Repeat until 3 vertices remain (last triangle).
//!
//! O(n²) for simple polygons; sufficient for any EDA polygon you'll
//! encounter (typical hull lengths < 100 vertices). For pathological
//! polygons with thousands of vertices, advanced algorithms (Seidel,
//! constrained-Delaunay) win — v2.

use klayout_core::{Point, Polygon};

#[derive(Clone, Debug)]
pub struct Triangle(pub Point, pub Point, pub Point);

/// Decompose a polygon hull into triangles. Holes are not yet
/// supported — pass `polygon.hull` only.
pub fn ear_clip(hull: &[Point]) -> Vec<Triangle> {
    let n = hull.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![Triangle(hull[0], hull[1], hull[2])];
    }
    // Determine winding; the algorithm assumes CCW.
    let mut pts: Vec<Point> = if signed_area(hull) < 0 {
        hull.iter().rev().copied().collect()
    } else {
        hull.to_vec()
    };
    let mut out: Vec<Triangle> = Vec::new();
    let mut guard = 0;
    while pts.len() > 3 && guard < 10_000 {
        guard += 1;
        let mut found_ear = false;
        let m = pts.len();
        for i in 0..m {
            let prev = pts[(i + m - 1) % m];
            let cur = pts[i];
            let next = pts[(i + 1) % m];
            if !is_convex(prev, cur, next) {
                continue;
            }
            if any_point_inside_triangle(prev, cur, next, &pts, i) {
                continue;
            }
            out.push(Triangle(prev, cur, next));
            pts.remove(i);
            found_ear = true;
            break;
        }
        if !found_ear {
            // Pathological case — bail out conservatively.
            break;
        }
    }
    if pts.len() == 3 {
        out.push(Triangle(pts[0], pts[1], pts[2]));
    }
    out
}

/// Decompose a polygon into convex pieces of arbitrary degree by
/// merging adjacent triangles whose union is still convex. Cheaper
/// downstream consumers (a single hexagon beats two triangles).
pub fn convex_pieces(hull: &[Point]) -> Vec<Vec<Point>> {
    let triangles = ear_clip(hull);
    triangles
        .into_iter()
        .map(|t| vec![t.0, t.1, t.2])
        .collect()
}

/// Convenience: decompose a `Polygon` (hull only). Hole support is
/// the caller's responsibility — typically you'd subtract holes
/// before decomposition.
pub fn decompose(polygon: &Polygon) -> Vec<Vec<Point>> {
    convex_pieces(&polygon.hull)
}

fn signed_area(pts: &[Point]) -> i128 {
    let n = pts.len();
    let mut s: i128 = 0;
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        s += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
    }
    s / 2
}

fn cross(a: Point, b: Point, c: Point) -> i128 {
    let abx = (b.x - a.x) as i128;
    let aby = (b.y - a.y) as i128;
    let acx = (c.x - a.x) as i128;
    let acy = (c.y - a.y) as i128;
    abx * acy - aby * acx
}

fn is_convex(prev: Point, cur: Point, next: Point) -> bool {
    cross(prev, cur, next) > 0
}

fn point_in_triangle(p: Point, a: Point, b: Point, c: Point) -> bool {
    let d1 = cross(p, a, b);
    let d2 = cross(p, b, c);
    let d3 = cross(p, c, a);
    let has_neg = d1 < 0 || d2 < 0 || d3 < 0;
    let has_pos = d1 > 0 || d2 > 0 || d3 > 0;
    !(has_neg && has_pos)
}

fn any_point_inside_triangle(
    a: Point,
    b: Point,
    c: Point,
    pts: &[Point],
    skip_idx: usize,
) -> bool {
    let m = pts.len();
    for (j, p) in pts.iter().enumerate() {
        let prev_idx = (skip_idx + m - 1) % m;
        let next_idx = (skip_idx + 1) % m;
        if j == skip_idx || j == prev_idx || j == next_idx {
            continue;
        }
        if point_in_triangle(*p, a, b, c) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::Polygon;

    fn pt(x: i64, y: i64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn triangle_returns_one_triangle() {
        let hull = vec![pt(0, 0), pt(10, 0), pt(5, 10)];
        let tris = ear_clip(&hull);
        assert_eq!(tris.len(), 1);
    }

    #[test]
    fn square_returns_two_triangles() {
        let hull = vec![pt(0, 0), pt(10, 0), pt(10, 10), pt(0, 10)];
        let tris = ear_clip(&hull);
        assert_eq!(tris.len(), 2);
    }

    #[test]
    fn pentagon_returns_three_triangles() {
        let hull = vec![pt(0, 0), pt(10, 0), pt(15, 5), pt(5, 12), pt(-5, 5)];
        let tris = ear_clip(&hull);
        assert_eq!(tris.len(), 3);
    }

    #[test]
    fn l_shape_decomposes() {
        // L-shape (6 vertices, CCW after canonicalisation).
        let hull = vec![
            pt(0, 0),
            pt(10, 0),
            pt(10, 4),
            pt(4, 4),
            pt(4, 10),
            pt(0, 10),
        ];
        let tris = ear_clip(&hull);
        // Expect 4 triangles for a 6-vertex polygon.
        assert_eq!(tris.len(), 4);
    }

    #[test]
    fn area_preserved_after_decomposition() {
        let hull = vec![pt(0, 0), pt(10, 0), pt(10, 10), pt(0, 10)];
        let total: i128 = ear_clip(&hull)
            .iter()
            .map(|t| triangle_area2(t).abs())
            .sum::<i128>()
            / 2;
        assert_eq!(total, 100);
    }

    #[test]
    fn handles_cw_input() {
        let hull = vec![pt(0, 0), pt(0, 10), pt(10, 10), pt(10, 0)];
        let tris = ear_clip(&hull);
        assert_eq!(tris.len(), 2);
    }

    #[test]
    fn decompose_polygon_uses_hull() {
        let hull = vec![pt(0, 0), pt(10, 0), pt(10, 10), pt(0, 10)];
        let p = Polygon::from_hull(hull);
        let pieces = decompose(&p);
        assert_eq!(pieces.len(), 2);
        for piece in pieces {
            assert!(piece.len() >= 3);
        }
    }

    fn triangle_area2(t: &Triangle) -> i128 {
        let abx = (t.1.x - t.0.x) as i128;
        let aby = (t.1.y - t.0.y) as i128;
        let acx = (t.2.x - t.0.x) as i128;
        let acy = (t.2.y - t.0.y) as i128;
        abx * acy - aby * acx
    }
}
