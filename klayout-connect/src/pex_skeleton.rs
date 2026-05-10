//! Skeleton-based polygon resistance for PEX.
//!
//! The bbox aspect-ratio resistance in [`pex.rs`] is fast but
//! pessimistic for L-shapes, T-shapes, and bent wires. Real PEX
//! computes resistance from the polygon's *medial axis* (skeleton):
//! a 1-D path through the polygon centerline. The resistance is then
//! `sum(segment_length / segment_width × sheet_rho)` where
//! `segment_width` is twice the perpendicular distance from the
//! skeleton to the polygon boundary.
//!
//! v1 implements a simplified rectangle-decomposition skeleton:
//! 1. Decompose the polygon into a set of axis-aligned rectangles
//!    (trivial for already-rectangular wires; greedy slicing for
//!    L/T shapes).
//! 2. The skeleton of each rectangle is its centerline along the
//!    longer dimension.
//! 3. At rectangle joins, connect skeletons with their endpoint
//!    centerlines.
//!
//! This is exact for rectilinear shapes and approximate-but-good
//! for the bent-wire cases the bbox model fails on. Full medial-
//! axis transform (Voronoi-based) is a v2.

use crate::pex::LayerPexParams;
use klayout_core::{Bbox, Point, Polygon};

/// Compute the skeleton-based resistance of a polygon (Ω).
///
/// For pure rectangles this gives `sheet_rho × (long / short)`,
/// matching the bbox model. For L-shapes / T-shapes, it sums the
/// resistances of each constituent rectangle (decomposed by axis-
/// aligned slicing) — capturing the bend penalty the bbox model
/// misses.
pub fn skeleton_resistance(polygon: &Polygon, params: &LayerPexParams) -> f64 {
    if polygon.hull.is_empty() {
        return 0.0;
    }
    let rects = decompose_rects(polygon);
    if rects.is_empty() {
        return 0.0;
    }
    let mut total = 0.0;
    for r in &rects {
        let w = r.width().max(1) as f64;
        let h = r.height().max(1) as f64;
        let long = w.max(h);
        let short = w.min(h);
        total += params.sheet_rho * long / short;
    }
    total
}

/// Decompose a polygon into axis-aligned bounding rectangles.
/// Production EDA uses sweep-line trapezoidalization; for v1 we
/// slice by interior x-coordinates of vertices, which gives an
/// exact decomposition for rectilinear polygons.
pub fn decompose_rects(polygon: &Polygon) -> Vec<Bbox> {
    if polygon.hull.is_empty() {
        return Vec::new();
    }
    let bbox = polygon.bbox();
    if !is_rectilinear(polygon) {
        // Non-rectilinear → fall back to single bbox.
        return vec![bbox];
    }
    let mut x_coords: Vec<i64> = polygon.hull.iter().map(|p| p.x).collect();
    x_coords.sort();
    x_coords.dedup();
    if x_coords.len() < 2 {
        return vec![bbox];
    }
    let mut rects = Vec::new();
    for w in x_coords.windows(2) {
        let (x_lo, x_hi) = (w[0], w[1]);
        let mid_x = (x_lo + x_hi) / 2;
        // Find the vertical extent at this x by ray-casting.
        if let Some((y_lo, y_hi)) = vertical_extent(polygon, mid_x) {
            rects.push(Bbox::new(
                Point::new(x_lo, y_lo),
                Point::new(x_hi, y_hi),
            ));
        }
    }
    rects
}

/// Skeleton path: centerline through each decomposed rectangle.
/// Returns a list of `(start, end)` segments approximating the
/// medial axis. For visualization / via-array placement.
pub fn skeleton_path(polygon: &Polygon) -> Vec<(Point, Point)> {
    let rects = decompose_rects(polygon);
    let mut out = Vec::new();
    for r in &rects {
        let w = r.width();
        let h = r.height();
        if w >= h {
            // Horizontal centerline.
            let y = (r.min.y + r.max.y) / 2;
            out.push((Point::new(r.min.x, y), Point::new(r.max.x, y)));
        } else {
            let x = (r.min.x + r.max.x) / 2;
            out.push((Point::new(x, r.min.y), Point::new(x, r.max.y)));
        }
    }
    out
}

fn is_rectilinear(polygon: &Polygon) -> bool {
    let n = polygon.hull.len();
    if n < 4 {
        return false;
    }
    for i in 0..n {
        let a = polygon.hull[i];
        let b = polygon.hull[(i + 1) % n];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        if dx != 0 && dy != 0 {
            return false;
        }
    }
    true
}

/// At a given vertical line `x`, find the lo/hi y range of the
/// polygon's interior. Returns `None` if the line doesn't
/// intersect the polygon.
fn vertical_extent(polygon: &Polygon, x: i64) -> Option<(i64, i64)> {
    let bbox = polygon.bbox();
    if x < bbox.min.x || x > bbox.max.x {
        return None;
    }
    let mut crossings: Vec<i64> = Vec::new();
    let n = polygon.hull.len();
    for i in 0..n {
        let a = polygon.hull[i];
        let b = polygon.hull[(i + 1) % n];
        // Vertical edge at this x?
        if a.x == x && b.x == x {
            crossings.push(a.y);
            crossings.push(b.y);
        } else if (a.x < x) != (b.x < x) {
            // Horizontal edge crossing x somewhere in between.
            if a.y == b.y {
                crossings.push(a.y);
            } else {
                let t = (x - a.x) as f64 / (b.x - a.x) as f64;
                let y = a.y as f64 + t * (b.y - a.y) as f64;
                crossings.push(y.round() as i64);
            }
        }
    }
    crossings.sort();
    let lo = *crossings.first()?;
    let hi = *crossings.last()?;
    Some((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Polygon {
        Polygon::rect(Bbox::new(Point::new(x0, y0), Point::new(x1, y1)))
    }

    fn params() -> LayerPexParams {
        LayerPexParams {
            sheet_rho: 0.1,
            area_cap: 0.0,
            edge_cap: 0.0,
            coupling_max_distance_dbu: 0,
            coupling_cap: 0.0,
            dbu_per_um: 1000.0,
        }
    }

    #[test]
    fn rectangle_resistance_matches_bbox_formula() {
        // 100×10 rectangle: long/short = 10. R = 0.1 × 10 = 1.0.
        let p = rect(0, 0, 100, 10);
        let r = skeleton_resistance(&p, &params());
        assert!((r - 1.0).abs() < 1e-9);
    }

    #[test]
    fn l_shape_decomposes_to_two_rects() {
        // L-shape: (0,0)-(100,10) horizontal + (0,0)-(10,100) vertical.
        // Built by manually constructing the hull.
        let hull = vec![
            Point::new(0, 0),
            Point::new(0, 100),
            Point::new(10, 100),
            Point::new(10, 10),
            Point::new(100, 10),
            Point::new(100, 0),
        ];
        let p = Polygon::from_hull(hull);
        let rects = decompose_rects(&p);
        assert!(rects.len() >= 2);
    }

    #[test]
    fn skeleton_path_uses_long_axis() {
        let p = rect(0, 0, 100, 10);
        let segs = skeleton_path(&p);
        assert_eq!(segs.len(), 1);
        let (a, b) = segs[0];
        // Horizontal centerline: y = 5.
        assert_eq!(a.y, 5);
        assert_eq!(b.y, 5);
        assert_eq!(a.x, 0);
        assert_eq!(b.x, 100);
    }

    #[test]
    fn non_rectilinear_falls_back_to_bbox() {
        let hull = vec![
            Point::new(0, 0),
            Point::new(100, 50),
            Point::new(50, 100),
        ];
        let p = Polygon::from_hull(hull);
        let rects = decompose_rects(&p);
        assert_eq!(rects.len(), 1);
    }
}
