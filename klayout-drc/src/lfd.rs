//! Lithography-Friendly-Design (LFD) checks.
//!
//! Beyond the geometric DRC rules, foundries flag *patterns* that
//! print poorly: line ends near corners, jogs that create hot-spots,
//! tight U-shapes that resolve as bridges. These pattern checks live
//! in this module — each check scans the layout for a specific
//! problematic geometry and emits violations.
//!
//! v1 ships three classic LFD checks:
//!
//! * [`line_end_near_corner`] — wire endpoint within `dist` of a
//!   corner on the same layer. The proximity narrows the
//!   process window and resolution can fall below printability.
//! * [`tight_u_shape`] — narrow `U` (parallel-run + connecting jog)
//!   where `width < threshold`. Bridges across U-shapes are a
//!   classic litho-bridge mode.
//! * [`small_jog`] — single-step jog under `min_jog_length`. Jogs
//!   shorter than the resolution limit smear together.
//!
//! Each check returns a `Region` of violation locations (small
//! markers around the offending geometry).

use klayout_core::{Bbox, Point, Polygon};
use klayout_geom::{merge, Edges, Region};

/// Find every wire endpoint on `r` whose nearest corner on `r`
/// (excluding its own polygon's corners?) sits within `dist`.
///
/// v1 implementation: collect all polygon endpoints (vertices on the
/// hull); pair-wise distance check; flag any pair with `dist < threshold`
/// and `dist > 0` (skip zero-distance which means same vertex).
pub fn line_end_near_corner(r: &Region, dist: i64) -> Region {
    if r.is_empty() || dist <= 0 {
        return Region::empty();
    }
    let merged = merge(r);
    let mut corners: Vec<Point> = Vec::new();
    for p in merged.polygons() {
        for v in &p.hull {
            corners.push(*v);
        }
    }
    let dist2 = (dist as i128) * (dist as i128);
    let mut markers: Vec<Polygon> = Vec::new();
    for i in 0..corners.len() {
        for j in (i + 1)..corners.len() {
            let dx = (corners[i].x - corners[j].x) as i128;
            let dy = (corners[i].y - corners[j].y) as i128;
            let d2 = dx * dx + dy * dy;
            if d2 > 0 && d2 < dist2 {
                let mid = Point::new(
                    (corners[i].x + corners[j].x) / 2,
                    (corners[i].y + corners[j].y) / 2,
                );
                let half = (dist / 4).max(1);
                markers.push(Polygon::rect(Bbox::new(
                    Point::new(mid.x - half, mid.y - half),
                    Point::new(mid.x + half, mid.y + half),
                )));
            }
        }
    }
    if markers.is_empty() {
        Region::empty()
    } else {
        merge(&Region::from_polygons(markers))
    }
}

/// Detect tight U-shapes: parallel horizontal+vertical+horizontal
/// edge triples on the same polygon hull where the perpendicular
/// distance between the two parallel edges is below `width_threshold`.
pub fn tight_u_shape(r: &Region, width_threshold: i64) -> Region {
    if r.is_empty() || width_threshold <= 0 {
        return Region::empty();
    }
    let merged = merge(r);
    let mut markers: Vec<Polygon> = Vec::new();
    for p in merged.polygons() {
        let n = p.hull.len();
        if n < 4 {
            continue;
        }
        for i in 0..n {
            let a = p.hull[i];
            let b = p.hull[(i + 1) % n];
            let c = p.hull[(i + 2) % n];
            let d = p.hull[(i + 3) % n];
            let ab_h = a.y == b.y && a.x != b.x;
            let bc_v = b.x == c.x && b.y != c.y;
            let cd_h = c.y == d.y && c.x != d.x;
            let ab_v = a.x == b.x && a.y != b.y;
            let bc_h = b.y == c.y && b.x != c.x;
            let cd_v = c.x == d.x && c.y != d.y;
            let pattern_hvh = ab_h && bc_v && cd_h;
            let pattern_vhv = ab_v && bc_h && cd_v;
            if !(pattern_hvh || pattern_vhv) {
                continue;
            }
            // U "opens" iff the first and third legs run in opposite
            // directions along the parallel axis.
            let opens = if pattern_hvh {
                (b.x - a.x).signum() != (d.x - c.x).signum()
            } else {
                (b.y - a.y).signum() != (d.y - c.y).signum()
            };
            if !opens {
                continue;
            }
            let gap = if pattern_hvh {
                (b.y - c.y).abs()
            } else {
                (b.x - c.x).abs()
            };
            if gap > 0 && gap < width_threshold {
                let mid_x = (a.x + d.x) / 2;
                let mid_y = (a.y + d.y) / 2;
                let half = (width_threshold / 2).max(1);
                markers.push(Polygon::rect(Bbox::new(
                    Point::new(mid_x - half, mid_y - half),
                    Point::new(mid_x + half, mid_y + half),
                )));
            }
        }
    }
    if markers.is_empty() {
        Region::empty()
    } else {
        merge(&Region::from_polygons(markers))
    }
}

/// Detect small jogs: edges shorter than `min_jog_length` whose two
/// neighbors run perpendicular to it. Common litho problem on tight
/// staircases.
pub fn small_jog(r: &Region, min_jog_length: i64) -> Region {
    if r.is_empty() || min_jog_length <= 0 {
        return Region::empty();
    }
    let edges = Edges::from_region(r);
    let mut markers: Vec<Polygon> = Vec::new();
    for e in edges.edges() {
        let len2 = e.length_squared();
        let limit_sq = (min_jog_length as i128) * (min_jog_length as i128);
        if len2 > 0 && len2 < limit_sq && e.is_axis_aligned() {
            let mid = e.midpoint();
            let half = min_jog_length.max(1) / 2;
            markers.push(Polygon::rect(Bbox::new(
                Point::new(mid.x - half, mid.y - half),
                Point::new(mid.x + half, mid.y + half),
            )));
        }
    }
    if markers.is_empty() {
        Region::empty()
    } else {
        merge(&Region::from_polygons(markers))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Polygon {
        Polygon::rect(Bbox::new(Point::new(x0, y0), Point::new(x1, y1)))
    }

    #[test]
    fn no_corners_no_violation() {
        let r = Region::empty();
        assert!(line_end_near_corner(&r, 10).is_empty());
        assert!(tight_u_shape(&r, 10).is_empty());
        assert!(small_jog(&r, 5).is_empty());
    }

    #[test]
    fn close_corners_flagged() {
        // Two rects with corners 3 DBU apart.
        let r = Region::from_polygons([rect(0, 0, 10, 10), rect(13, 0, 23, 10)]);
        let v = line_end_near_corner(&r, 5);
        assert!(!v.is_empty());
    }

    #[test]
    fn tight_u_detected() {
        // U-shape: 0..20 wide × 30 tall with a 4-DBU notch.
        let pts = vec![
            Point::new(0, 0),
            Point::new(20, 0),
            Point::new(20, 30),
            Point::new(12, 30),
            Point::new(12, 4),  // bottom of slot
            Point::new(8, 4),   // bottom of slot
            Point::new(8, 30),  // top side of slot
            Point::new(0, 30),
        ];
        let r = Region::from_polygons([Polygon::from_hull(pts)]);
        let v = tight_u_shape(&r, 10);
        assert!(!v.is_empty(), "tight U should be flagged");
    }

    #[test]
    fn small_jog_detected_on_thin_edge() {
        // Polygon with a 1-DBU jog edge.
        let pts = vec![
            Point::new(0, 0),
            Point::new(50, 0),
            Point::new(50, 1),  // 1-unit jog
            Point::new(60, 1),
            Point::new(60, 10),
            Point::new(0, 10),
        ];
        let r = Region::from_polygons([Polygon::from_hull(pts)]);
        let v = small_jog(&r, 5);
        assert!(!v.is_empty());
    }
}
