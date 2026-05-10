//! Mask-data fracturing — trapezoidalisation.
//!
//! Mask-write tools (electron-beam, laser, multi-beam) consume a
//! polygon as a list of axis-aligned trapezoids: shapes with two
//! horizontal sides and (possibly slanted) left/right sides. The
//! fracture pass splits an arbitrary polygon into this canonical
//! form so the writer can stripe-scan it linearly.
//!
//! Algorithm: horizontal scanline.
//! 1. Collect the unique y-coordinates from the polygon's vertices.
//!    These are the horizontal split lines.
//! 2. For each pair of consecutive y-values `(y_lo, y_hi)`, find the
//!    polygon's intersection with the strip `y ∈ [y_lo, y_hi]`. The
//!    intersection is one (or more) trapezoids whose top and bottom
//!    sides sit at y_lo and y_hi.
//! 3. Emit each trapezoid as a 4-vertex polygon.
//!
//! v1 handles single-hull polygons (no holes) via a sweep over
//! horizontal strips. Holes can be approximated by subtracting the
//! hole's trapezoids from the hull's set.

use klayout_core::{Point, Polygon};

/// One axis-aligned trapezoid: two horizontal sides at `y_lo` / `y_hi`,
/// left side from `(x_lo_bot, y_lo)` to `(x_lo_top, y_hi)`,
/// right side from `(x_hi_bot, y_lo)` to `(x_hi_top, y_hi)`.
#[derive(Clone, Debug, PartialEq)]
pub struct Trapezoid {
    pub y_lo: i64,
    pub y_hi: i64,
    pub x_lo_bot: i64,
    pub x_hi_bot: i64,
    pub x_lo_top: i64,
    pub x_hi_top: i64,
}

impl Trapezoid {
    pub fn to_polygon(&self) -> Polygon {
        let pts = vec![
            Point::new(self.x_lo_bot, self.y_lo),
            Point::new(self.x_hi_bot, self.y_lo),
            Point::new(self.x_hi_top, self.y_hi),
            Point::new(self.x_lo_top, self.y_hi),
        ];
        Polygon::from_hull(pts)
    }
}

/// Fracture a polygon hull into trapezoids. Returns one trapezoid
/// per horizontal strip of the polygon.
pub fn fracture(polygon: &Polygon) -> Vec<Trapezoid> {
    let hull = &polygon.hull;
    if hull.len() < 3 {
        return Vec::new();
    }
    // Collect unique y coordinates.
    let mut ys: Vec<i64> = hull.iter().map(|p| p.y).collect();
    ys.sort();
    ys.dedup();
    if ys.len() < 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for w in ys.windows(2) {
        let y_lo = w[0];
        let y_hi = w[1];
        let strips = strip_intersections(hull, y_lo, y_hi);
        out.extend(strips);
    }
    out
}

/// For one horizontal strip, find the trapezoidal pieces.
fn strip_intersections(hull: &[Point], y_lo: i64, y_hi: i64) -> Vec<Trapezoid> {
    let n = hull.len();
    // Compute x-intersections of every polygon edge with both
    // horizontal sweep lines.
    let mut bot_xs: Vec<i64> = Vec::new();
    let mut top_xs: Vec<i64> = Vec::new();
    for i in 0..n {
        let a = hull[i];
        let b = hull[(i + 1) % n];
        // Edge crosses the strip iff its y range straddles either
        // boundary.
        if let Some(x) = line_at_y(a, b, y_lo) {
            bot_xs.push(x);
        }
        if let Some(x) = line_at_y(a, b, y_hi) {
            top_xs.push(x);
        }
    }
    bot_xs.sort();
    top_xs.sort();

    // Pair up bot and top crossings; emit one trapezoid per pair.
    // For a simple polygon, both lists should have the same even
    // count (entrance/exit pairs).
    let pairs = bot_xs.len().min(top_xs.len()) / 2;
    let mut out = Vec::with_capacity(pairs);
    for i in 0..pairs {
        let x_lo_bot = bot_xs[2 * i];
        let x_hi_bot = bot_xs[2 * i + 1];
        let x_lo_top = top_xs[2 * i];
        let x_hi_top = top_xs[2 * i + 1];
        if x_lo_bot == x_hi_bot && x_lo_top == x_hi_top {
            continue; // degenerate strip
        }
        out.push(Trapezoid {
            y_lo,
            y_hi,
            x_lo_bot,
            x_hi_bot,
            x_lo_top,
            x_hi_top,
        });
    }
    out
}

/// Find the x-coordinate where the line through `a-b` crosses
/// `y = y_target`. Returns `None` if the edge doesn't include `y_target`
/// in its (closed) y-range, or if the edge is horizontal.
fn line_at_y(a: Point, b: Point, y_target: i64) -> Option<i64> {
    let y_min = a.y.min(b.y);
    let y_max = a.y.max(b.y);
    if y_target < y_min || y_target > y_max {
        return None;
    }
    if a.y == b.y {
        return None;
    }
    let t = (y_target - a.y) as f64 / (b.y - a.y) as f64;
    let x = a.x as f64 + t * (b.x - a.x) as f64;
    Some(x.round() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: i64, y: i64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn rectangle_fractures_to_one_trapezoid() {
        let p = Polygon::from_hull(vec![pt(0, 0), pt(10, 0), pt(10, 10), pt(0, 10)]);
        let traps = fracture(&p);
        assert_eq!(traps.len(), 1);
        assert_eq!(traps[0].y_lo, 0);
        assert_eq!(traps[0].y_hi, 10);
        assert_eq!(traps[0].x_lo_bot, 0);
        assert_eq!(traps[0].x_hi_bot, 10);
    }

    #[test]
    fn triangle_fractures_to_one_trapezoid() {
        // Triangle with apex on top.
        let p = Polygon::from_hull(vec![pt(0, 0), pt(10, 0), pt(5, 10)]);
        let traps = fracture(&p);
        assert_eq!(traps.len(), 1);
        // Top sides converge.
        assert_eq!(traps[0].x_lo_top, traps[0].x_hi_top);
    }

    #[test]
    fn l_shape_fractures_to_two_strips() {
        // L-shape: y ranges 0..4 (full width 0..10) and 4..10 (width 0..4).
        let p = Polygon::from_hull(vec![
            pt(0, 0),
            pt(10, 0),
            pt(10, 4),
            pt(4, 4),
            pt(4, 10),
            pt(0, 10),
        ]);
        let traps = fracture(&p);
        assert_eq!(traps.len(), 2);
    }

    #[test]
    fn trapezoid_round_trips_to_polygon() {
        let t = Trapezoid {
            y_lo: 0,
            y_hi: 10,
            x_lo_bot: 0,
            x_hi_bot: 20,
            x_lo_top: 5,
            x_hi_top: 15,
        };
        let p = t.to_polygon();
        assert_eq!(p.hull.len(), 4);
    }

    #[test]
    fn empty_polygon_yields_no_trapezoids() {
        let p = Polygon::from_hull(vec![pt(0, 0)]);
        assert!(fracture(&p).is_empty());
    }
}
