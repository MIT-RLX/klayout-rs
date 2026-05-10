//! Edge collections — first-class edges as a complement to `Region`.
//!
//! KLayout treats edges as their own type with a rich operator set:
//! `centers`, `extended`, `with_angle`, `interacting`, `length_at_least`,
//! and so on. Many DRC primitives consume an `Edges` rather than a
//! `Region` (e.g. measure spacing only on edges of a particular angle).
//!
//! Edges carry a direction (a → b). For polygon-derived edges that
//! direction follows the hull's traversal order (CW for the outer
//! boundary). Filter operators preserve the source direction.
//!
//! v1 covers axis-aligned and arbitrary-angle edges. Some operators
//! (`outside_part`, `inside_part`) are implemented for axis-aligned
//! input; non-axis-aligned cases fall back to bbox-based filtering.

use crate::Region;
use klayout_core::{Bbox, Point, Polygon};

/// One directed edge from `a` to `b`. Edges with `a == b` are valid but
/// don't contribute to most operators (they're zero-length).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct Edge {
    pub a: Point,
    pub b: Point,
}

impl Edge {
    pub const fn new(a: Point, b: Point) -> Self {
        Self { a, b }
    }

    pub fn dx(&self) -> i64 {
        self.b.x - self.a.x
    }

    pub fn dy(&self) -> i64 {
        self.b.y - self.a.y
    }

    pub fn length_squared(&self) -> i128 {
        let dx = self.dx() as i128;
        let dy = self.dy() as i128;
        dx * dx + dy * dy
    }

    pub fn length(&self) -> f64 {
        (self.length_squared() as f64).sqrt()
    }

    pub fn midpoint(&self) -> Point {
        Point::new((self.a.x + self.b.x) / 2, (self.a.y + self.b.y) / 2)
    }

    pub fn bbox(&self) -> Bbox {
        Bbox::new(
            Point::new(self.a.x.min(self.b.x), self.a.y.min(self.b.y)),
            Point::new(self.a.x.max(self.b.x), self.a.y.max(self.b.y)),
        )
    }

    /// Angle in degrees from the +X axis, normalized to `[0, 180)`.
    /// Reverse-direction edges have the same angle as their forward
    /// counterpart — useful for "edges parallel to direction X".
    pub fn angle_degrees(&self) -> f64 {
        let mut a = (self.dy() as f64).atan2(self.dx() as f64).to_degrees();
        if a < 0.0 {
            a += 180.0;
        }
        if a >= 180.0 {
            a -= 180.0;
        }
        a
    }

    pub fn is_horizontal(&self) -> bool {
        self.dy() == 0 && self.dx() != 0
    }

    pub fn is_vertical(&self) -> bool {
        self.dx() == 0 && self.dy() != 0
    }

    pub fn is_axis_aligned(&self) -> bool {
        self.is_horizontal() || self.is_vertical()
    }
}

#[derive(Default, Clone, Debug)]
pub struct Edges {
    edges: Vec<Edge>,
}

impl Edges {
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_edges<I: IntoIterator<Item = Edge>>(it: I) -> Self {
        Self {
            edges: it.into_iter().collect(),
        }
    }

    /// Edges of a single polygon — the hull plus every hole boundary,
    /// in traversal order.
    pub fn from_polygon(p: &Polygon) -> Self {
        let mut edges = Vec::new();
        push_loop(&mut edges, &p.hull);
        for hole in &p.holes {
            push_loop(&mut edges, hole);
        }
        Self { edges }
    }

    /// Edges of every polygon in a region.
    pub fn from_region(r: &Region) -> Self {
        let mut edges = Vec::new();
        for p in r.polygons() {
            push_loop(&mut edges, &p.hull);
            for hole in &p.holes {
                push_loop(&mut edges, hole);
            }
        }
        Self { edges }
    }

    pub fn len(&self) -> usize {
        self.edges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    pub fn into_inner(self) -> Vec<Edge> {
        self.edges
    }

    /// Replace each edge with a `length`-long segment centered at its
    /// midpoint, oriented along the source edge. Edges shorter than
    /// `length` are emitted unchanged.
    pub fn centers(&self, length: i64) -> Edges {
        if length <= 0 {
            return Edges::empty();
        }
        let mut out = Vec::with_capacity(self.edges.len());
        for e in &self.edges {
            let len2 = e.length_squared();
            if len2 == 0 {
                continue;
            }
            let len = (len2 as f64).sqrt();
            if (len as i64) <= length {
                out.push(*e);
                continue;
            }
            let t = length as f64 / (2.0 * len);
            let mid = e.midpoint();
            let dx = e.dx() as f64;
            let dy = e.dy() as f64;
            let ax = mid.x as f64 - dx * t;
            let ay = mid.y as f64 - dy * t;
            let bx = mid.x as f64 + dx * t;
            let by = mid.y as f64 + dy * t;
            out.push(Edge::new(
                Point::new(ax.round() as i64, ay.round() as i64),
                Point::new(bx.round() as i64, by.round() as i64),
            ));
        }
        Edges::from_edges(out)
    }

    /// Extend each edge by `before` along the −direction at `a` and by
    /// `after` along the +direction at `b`. Useful for converting an
    /// edge to a "ray" with finite extent.
    pub fn extended(&self, before: i64, after: i64) -> Edges {
        let mut out = Vec::with_capacity(self.edges.len());
        for e in &self.edges {
            let len2 = e.length_squared();
            if len2 == 0 {
                out.push(*e);
                continue;
            }
            let len = (len2 as f64).sqrt();
            let dx = e.dx() as f64 / len;
            let dy = e.dy() as f64 / len;
            let ax = (e.a.x as f64) - dx * (before as f64);
            let ay = (e.a.y as f64) - dy * (before as f64);
            let bx = (e.b.x as f64) + dx * (after as f64);
            let by = (e.b.y as f64) + dy * (after as f64);
            out.push(Edge::new(
                Point::new(ax.round() as i64, ay.round() as i64),
                Point::new(bx.round() as i64, by.round() as i64),
            ));
        }
        Edges::from_edges(out)
    }

    /// Filter edges whose angle (mod 180°) is within `tolerance_deg` of
    /// `angle_deg`.
    pub fn with_angle(&self, angle_deg: f64, tolerance_deg: f64) -> Edges {
        Edges::from_edges(self.edges.iter().copied().filter(|e| {
            if e.length_squared() == 0 {
                return false;
            }
            angles_close(e.angle_degrees(), angle_deg, tolerance_deg)
        }))
    }

    pub fn not_with_angle(&self, angle_deg: f64, tolerance_deg: f64) -> Edges {
        Edges::from_edges(self.edges.iter().copied().filter(|e| {
            if e.length_squared() == 0 {
                return false;
            }
            !angles_close(e.angle_degrees(), angle_deg, tolerance_deg)
        }))
    }

    pub fn length_at_least(&self, min: i64) -> Edges {
        let m2 = (min as i128) * (min as i128);
        Edges::from_edges(
            self.edges
                .iter()
                .copied()
                .filter(|e| e.length_squared() >= m2),
        )
    }

    pub fn length_at_most(&self, max: i64) -> Edges {
        let m2 = (max as i128) * (max as i128);
        Edges::from_edges(
            self.edges
                .iter()
                .copied()
                .filter(|e| e.length_squared() <= m2),
        )
    }

    /// Keep edges whose bbox overlaps any edge in `other`. v1 uses bbox
    /// intersection — sufficient for axis-aligned edge interaction
    /// (where touching edges share a bbox boundary point).
    pub fn interacting(&self, other: &Edges) -> Edges {
        if self.is_empty() || other.is_empty() {
            return Edges::empty();
        }
        let other_bboxes: Vec<Bbox> = other.edges.iter().map(|e| e.bbox()).collect();
        Edges::from_edges(self.edges.iter().copied().filter(|e| {
            let eb = e.bbox();
            other_bboxes.iter().any(|ob| eb.intersects(ob))
        }))
    }

    /// Keep the parts of each edge that lie strictly outside `region`.
    /// Implemented for axis-aligned edges over axis-aligned regions —
    /// non-axis-aligned edges are returned unchanged.
    pub fn outside_part(&self, region: &Region) -> Edges {
        clip_edges(&self.edges, region, false)
    }

    /// Keep the parts of each edge that lie inside `region`.
    pub fn inside_part(&self, region: &Region) -> Edges {
        clip_edges(&self.edges, region, true)
    }
}

fn push_loop(out: &mut Vec<Edge>, pts: &[Point]) {
    let n = pts.len();
    if n < 2 {
        return;
    }
    for i in 0..n {
        let a = pts[i];
        let b = pts[(i + 1) % n];
        if a != b {
            out.push(Edge::new(a, b));
        }
    }
}

fn angles_close(a: f64, b: f64, tol: f64) -> bool {
    let mut diff = (a - b).abs() % 180.0;
    if diff > 90.0 {
        diff = 180.0 - diff;
    }
    diff <= tol
}

/// Clip every edge against `region`. `inside == true` keeps the inside
/// part; `inside == false` keeps the outside part.
///
/// For an axis-aligned edge (horizontal or vertical), we walk every
/// region polygon-edge that is parallel to the axis-aligned edge's
/// perpendicular and lies in its sweep range, computing crossings.
/// Non-axis-aligned input edges are passed through unchanged — KLayout
/// has a more elaborate algorithm that we leave for v2.
fn clip_edges(edges: &[Edge], region: &Region, inside: bool) -> Edges {
    if edges.is_empty() {
        return Edges::empty();
    }
    if region.is_empty() {
        return if inside {
            Edges::empty()
        } else {
            Edges::from_edges(edges.iter().copied())
        };
    }
    let mut out = Vec::with_capacity(edges.len());
    for e in edges {
        if !e.is_axis_aligned() {
            out.push(*e);
            continue;
        }
        let segments = clip_axis_aligned(e, region, inside);
        out.extend(segments);
    }
    Edges::from_edges(out)
}

fn clip_axis_aligned(e: &Edge, region: &Region, inside: bool) -> Vec<Edge> {
    let horizontal = e.is_horizontal();
    let (axis_lo, axis_hi) = if horizontal {
        let lo = e.a.x.min(e.b.x);
        let hi = e.a.x.max(e.b.x);
        (lo, hi)
    } else {
        let lo = e.a.y.min(e.b.y);
        let hi = e.a.y.max(e.b.y);
        (lo, hi)
    };
    let perp = if horizontal { e.a.y } else { e.a.x };
    let bbox = e.bbox();

    // Collect crossings: each region polygon edge that crosses our
    // sweep at perp coordinate `perp` contributes a split point.
    let mut splits: Vec<i64> = vec![axis_lo, axis_hi];
    for poly in region.polygons() {
        if !bbox.intersects(&poly.bbox()) {
            continue;
        }
        for loop_pts in std::iter::once(poly.hull.as_slice()).chain(poly.holes.iter().map(|h| h.as_slice())) {
            let n = loop_pts.len();
            for i in 0..n {
                let a = loop_pts[i];
                let b = loop_pts[(i + 1) % n];
                if let Some(x) = axis_aligned_crossing(a, b, perp, horizontal, axis_lo, axis_hi) {
                    splits.push(x);
                }
            }
        }
    }
    splits.sort();
    splits.dedup();

    let mut segments = Vec::new();
    for w in splits.windows(2) {
        let (s, t) = (w[0], w[1]);
        if s == t {
            continue;
        }
        let mid_axis = (s + t) / 2;
        let mid_point = if horizontal {
            Point::new(mid_axis, perp)
        } else {
            Point::new(perp, mid_axis)
        };
        let in_region = region_contains_point(region, mid_point);
        if inside == in_region {
            let p_a = if horizontal {
                Point::new(s, perp)
            } else {
                Point::new(perp, s)
            };
            let p_b = if horizontal {
                Point::new(t, perp)
            } else {
                Point::new(perp, t)
            };
            // Preserve original direction.
            let forward_axis = if horizontal { e.b.x - e.a.x } else { e.b.y - e.a.y } >= 0;
            if forward_axis {
                segments.push(Edge::new(p_a, p_b));
            } else {
                segments.push(Edge::new(p_b, p_a));
            }
        }
    }
    segments
}

/// If the segment `a → b` crosses the perpendicular sweep at coordinate
/// `perp` (y-coord if `horizontal`, else x-coord) and lands within the
/// edge's axis range `[lo, hi]`, return the crossing axis coordinate.
fn axis_aligned_crossing(
    a: Point,
    b: Point,
    perp: i64,
    horizontal: bool,
    lo: i64,
    hi: i64,
) -> Option<i64> {
    let (perp_a, perp_b, axis_a, axis_b) = if horizontal {
        (a.y, b.y, a.x, b.x)
    } else {
        (a.x, b.x, a.y, b.y)
    };
    if perp_a == perp_b {
        // Collinear — overlap range; return interior split points.
        if perp_a == perp {
            let lo_seg = axis_a.min(axis_b).max(lo);
            let hi_seg = axis_a.max(axis_b).min(hi);
            if lo_seg < hi_seg {
                // Two split points — both endpoints. Collinearity
                // causes both to be in `splits` already via lo/hi or
                // adjacent edges.
                return Some(lo_seg);
            }
        }
        return None;
    }
    if (perp_a < perp && perp_b < perp) || (perp_a > perp && perp_b > perp) {
        return None;
    }
    let t = (perp - perp_a) as f64 / (perp_b - perp_a) as f64;
    let axis = (axis_a as f64 + t * (axis_b as f64 - axis_a as f64)).round() as i64;
    if axis < lo || axis > hi {
        return None;
    }
    Some(axis)
}

fn region_contains_point(r: &Region, p: Point) -> bool {
    for poly in r.polygons() {
        if !poly.bbox().contains(p) {
            continue;
        }
        if point_in_polygon(p, &poly.hull) {
            let in_hole = poly.holes.iter().any(|h| point_in_polygon(p, h));
            if !in_hole {
                return true;
            }
        }
    }
    false
}

fn point_in_polygon(p: Point, ring: &[Point]) -> bool {
    // Even-odd ray-casting from `p` to `+x`. Robust enough for axis-
    // aligned and rotated polygons over integer coords.
    let mut inside = false;
    let n = ring.len();
    if n < 3 {
        return false;
    }
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        let crosses = (a.y > p.y) != (b.y > p.y);
        if crosses {
            let t = (p.y - a.y) as f64 / (b.y - a.y) as f64;
            let x_at = a.x as f64 + t * (b.x - a.x) as f64;
            if x_at > p.x as f64 {
                inside = !inside;
            }
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::Polygon;

    fn pt(x: i64, y: i64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn polygon_edges_are_directed() {
        let p = Polygon::rect(Bbox::new(pt(0, 0), pt(10, 5)));
        let e = Edges::from_polygon(&p);
        assert_eq!(e.len(), 4);
        // For a CCW rect (0,0) (10,0) (10,5) (0,5): edges are
        // east, north, west, south.
        let dirs: Vec<(i64, i64)> = e.edges().iter().map(|x| (x.dx(), x.dy())).collect();
        assert!(dirs.contains(&(10, 0)));
        assert!(dirs.contains(&(0, 5)));
        assert!(dirs.contains(&(-10, 0)));
        assert!(dirs.contains(&(0, -5)));
    }

    #[test]
    fn with_angle_filters_horizontals() {
        let p = Polygon::rect(Bbox::new(pt(0, 0), pt(10, 5)));
        let e = Edges::from_polygon(&p);
        let h = e.with_angle(0.0, 1.0);
        assert_eq!(h.len(), 2); // east + west
        let v = e.with_angle(90.0, 1.0);
        assert_eq!(v.len(), 2); // north + south
    }

    #[test]
    fn length_filter() {
        let p = Polygon::rect(Bbox::new(pt(0, 0), pt(10, 5)));
        let e = Edges::from_polygon(&p);
        assert_eq!(e.length_at_least(8).len(), 2); // only the 10-long edges
        assert_eq!(e.length_at_most(6).len(), 2); // only the 5-long edges
    }

    #[test]
    fn centers_emits_centered_segment() {
        let e = Edges::from_edges([Edge::new(pt(0, 0), pt(10, 0))]);
        let c = e.centers(2);
        assert_eq!(c.len(), 1);
        let seg = c.edges()[0];
        assert_eq!(seg.a, pt(4, 0));
        assert_eq!(seg.b, pt(6, 0));
    }

    #[test]
    fn extended_grows_endpoints() {
        let e = Edges::from_edges([Edge::new(pt(0, 0), pt(10, 0))]);
        let x = e.extended(2, 3);
        assert_eq!(x.edges()[0].a, pt(-2, 0));
        assert_eq!(x.edges()[0].b, pt(13, 0));
    }

    #[test]
    fn interacting_finds_overlapping_bboxes() {
        let a = Edges::from_edges([Edge::new(pt(0, 0), pt(10, 0))]);
        let b = Edges::from_edges([
            Edge::new(pt(5, 0), pt(5, 10)),
            Edge::new(pt(100, 100), pt(110, 100)),
        ]);
        let i = a.interacting(&b);
        assert_eq!(i.len(), 1);
    }

    #[test]
    fn outside_part_clips_against_region() {
        // Edge from (0,0) to (20,0); region covers x in [5,15].
        let e = Edges::from_edges([Edge::new(pt(0, 0), pt(20, 0))]);
        let r = Region::from_polygons([Polygon::rect(Bbox::new(pt(5, -5), pt(15, 5)))]);
        let out = e.outside_part(&r);
        // Two outside segments: [0,5] and [15,20].
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn inside_part_clips_against_region() {
        let e = Edges::from_edges([Edge::new(pt(0, 0), pt(20, 0))]);
        let r = Region::from_polygons([Polygon::rect(Bbox::new(pt(5, -5), pt(15, 5)))]);
        let inside = e.inside_part(&r);
        assert_eq!(inside.len(), 1);
        let seg = inside.edges()[0];
        assert_eq!(seg.bbox().min.x, 5);
        assert_eq!(seg.bbox().max.x, 15);
    }
}
