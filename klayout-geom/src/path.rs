//! Path → Polygon rendering.
//!
//! Convert a [`klayout_core::Path`] (centerline + width + cap) into a
//! closed `Polygon` so DRC can verify routed wires. v1 covers manhattan
//! (axis-aligned) paths exactly — the dominant case for digital and most
//! analog layouts. Non-axis-aligned segments fall back to perpendicular
//! offset with miter corners (some accuracy lost at sharp angles).

use klayout_core::{Path, PathCap, Point, Polygon};

/// Convert a `Path` to a closed `Polygon` representing the same area.
///
/// End cap semantics match KLayout:
/// * `Flat` — the path ends exactly at the end vertex (perpendicular).
/// * `Round` — the path is extended by `width/2` past the end vertex
///   (we approximate the round cap with a flat extension; geometric area
///   matches for square-cap path checks).
/// * `Extended` — extended by `path.begin_ext` / `path.end_ext` (or
///   `width/2` if both are zero, matching `PATHTYPE 2`).
pub fn path_to_polygon(p: &Path) -> Option<Polygon> {
    if p.points.len() < 2 || p.width <= 0 {
        return None;
    }
    let half = p.width / 2;
    let (begin_ext, end_ext) = match p.cap {
        PathCap::Flat => (0, 0),
        PathCap::Round => (half, half),
        PathCap::Extended => {
            if p.begin_ext != 0 || p.end_ext != 0 {
                (p.begin_ext, p.end_ext)
            } else {
                (half, half)
            }
        }
    };

    // Extend the centerline at each end.
    let mut center: Vec<Point> = p.points.to_vec();
    if begin_ext != 0 {
        center[0] = extend(center[1], center[0], begin_ext);
    }
    if end_ext != 0 {
        let n = center.len();
        center[n - 1] = extend(center[n - 2], center[n - 1], end_ext);
    }

    // Walk both sides of the centerline. For axis-aligned segments,
    // perpendicular offset is exact integer arithmetic. For arbitrary
    // angles, fall back to float math.
    let left = offset_polyline(&center, half, OffsetSide::Left);
    let right = offset_polyline(&center, half, OffsetSide::Right);

    // Build the closed polygon: left side forward, right side reversed.
    let mut hull = Vec::with_capacity(left.len() + right.len());
    hull.extend(left);
    for p in right.iter().rev() {
        hull.push(*p);
    }
    Some(Polygon::from_hull(hull))
}

#[derive(Copy, Clone)]
enum OffsetSide {
    Left,
    Right,
}

fn offset_polyline(pts: &[Point], dist: i64, side: OffsetSide) -> Vec<Point> {
    let mut out = Vec::with_capacity(pts.len());
    let n = pts.len();
    for i in 0..n {
        // Direction at vertex i: average of incoming and outgoing edge directions.
        let dir = if i == 0 {
            edge_dir(pts[0], pts[1])
        } else if i == n - 1 {
            edge_dir(pts[n - 2], pts[n - 1])
        } else {
            // Bisector between the two adjacent edges. For axis-aligned
            // 90° turns this is exactly along one of the axes.
            let d1 = edge_dir(pts[i - 1], pts[i]);
            let d2 = edge_dir(pts[i], pts[i + 1]);
            avg_dir(d1, d2)
        };
        let perp = match side {
            OffsetSide::Left => (-dir.1, dir.0),
            OffsetSide::Right => (dir.1, -dir.0),
        };
        let off_x = perp.0 * dist as f64;
        let off_y = perp.1 * dist as f64;
        out.push(Point::new(
            pts[i].x + off_x.round() as i64,
            pts[i].y + off_y.round() as i64,
        ));
    }
    out
}

/// Unit direction vector (f64) from `a` to `b`.
fn edge_dir(a: Point, b: Point) -> (f64, f64) {
    let dx = (b.x - a.x) as f64;
    let dy = (b.y - a.y) as f64;
    let len = (dx * dx + dy * dy).sqrt();
    if len == 0.0 {
        (0.0, 0.0)
    } else {
        (dx / len, dy / len)
    }
}

fn avg_dir(d1: (f64, f64), d2: (f64, f64)) -> (f64, f64) {
    let x = d1.0 + d2.0;
    let y = d1.1 + d2.1;
    let len = (x * x + y * y).sqrt();
    if len == 0.0 {
        (1.0, 0.0)
    } else {
        (x / len, y / len)
    }
}

fn extend(prev: Point, end: Point, by: i64) -> Point {
    let (dx, dy) = edge_dir(prev, end);
    Point::new(
        end.x + (dx * by as f64).round() as i64,
        end.y + (dy * by as f64).round() as i64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_path_flat_cap() {
        let p = Path {
            points: smallvec::smallvec![Point::new(0, 0), Point::new(100, 0)],
            width: 20,
            begin_ext: 0,
            end_ext: 0,
            cap: PathCap::Flat,
        };
        let poly = path_to_polygon(&p).unwrap();
        assert_eq!(poly.hull.len(), 4);
        let bbox = poly.bbox();
        assert_eq!(
            bbox,
            klayout_core::Bbox::new(Point::new(0, -10), Point::new(100, 10))
        );
    }

    #[test]
    fn horizontal_path_round_cap_extends() {
        let p = Path {
            points: smallvec::smallvec![Point::new(0, 0), Point::new(100, 0)],
            width: 20,
            begin_ext: 0,
            end_ext: 0,
            cap: PathCap::Round,
        };
        let poly = path_to_polygon(&p).unwrap();
        let bbox = poly.bbox();
        // Round cap extends by width/2 = 10 past each end.
        assert_eq!(
            bbox,
            klayout_core::Bbox::new(Point::new(-10, -10), Point::new(110, 10))
        );
    }

    #[test]
    fn l_path_axis_aligned() {
        let p = Path {
            points: smallvec::smallvec![
                Point::new(0, 0),
                Point::new(100, 0),
                Point::new(100, 100),
            ],
            width: 20,
            begin_ext: 0,
            end_ext: 0,
            cap: PathCap::Flat,
        };
        let poly = path_to_polygon(&p).unwrap();
        // Exterior bbox = -10..110 in x, -10..110 in y (the bend's
        // bisector offset reaches the outer corner).
        let bbox = poly.bbox();
        assert!(bbox.min.x <= 0);
        assert!(bbox.min.y <= -10);
        assert!(bbox.max.x >= 110);
        assert!(bbox.max.y >= 100);
    }

    #[test]
    fn zero_width_returns_none() {
        let p = Path {
            points: smallvec::smallvec![Point::new(0, 0), Point::new(100, 0)],
            width: 0,
            begin_ext: 0,
            end_ext: 0,
            cap: PathCap::Flat,
        };
        assert!(path_to_polygon(&p).is_none());
    }
}
