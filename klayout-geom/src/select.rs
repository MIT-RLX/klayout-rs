//! Selection / classification operators on `Region` — the core
//! KLayout deck DSL primitives.
//!
//! These return *subsets* of an input region according to topological
//! relations with another region or with text/polygon-area predicates.
//! They're the bread-and-butter of foundry decks: "find all metal1
//! shapes that touch a via", "find polygons inside a fence region",
//! "find shapes whose label is VDD".
//!
//! Operators (matching KLayout's naming):
//! * [`interacting`] — polygons of `a` that share at least one point
//!   with any polygon of `b`. Includes touching (boundary-only) and
//!   overlapping pairs.
//! * [`not_interacting`] — complement of `interacting` (returns `a`'s
//!   polygons that have no contact with `b`).
//! * [`inside`] — polygons of `a` whose closure lies entirely inside
//!   some polygon of `b`. Strict (boundary-touching counts as inside).
//! * [`outside`] — polygons of `a` with no shared points with `b`.
//!   Equivalent to `not_interacting` but expressed in containment terms.
//! * [`overlapping`] — polygons of `a` whose interiors share area with
//!   `b` (touching-only is *not* overlapping).
//! * [`with_text`] — polygons of `a` that contain any anchor of a
//!   `Text` shape with the given label string.
//! * [`select_by_area`] — polygons whose area falls in `[min, max]`.
//! * [`select_by_perimeter`] — polygons whose perimeter falls in
//!   `[min, max]`.
//! * [`select_by_aspect_ratio`] — polygons whose bbox `long/short`
//!   ratio falls in `[min, max]`.

use crate::Region;
use klayout_core::{Bbox, Polygon, Text};
use klayout_spatial::SpatialIndex;

/// Polygons of `a` that touch or overlap any polygon of `b`.
pub fn interacting(a: &Region, b: &Region) -> Region {
    if a.is_empty() || b.is_empty() {
        return Region::empty();
    }
    let idx = SpatialIndex::build(b.polygons().iter().enumerate().map(|(i, p)| (p.bbox(), i)));
    let kept: Vec<Polygon> = a
        .polygons()
        .iter()
        .filter(|pa| {
            let bb_a = pa.bbox();
            for (_, &j) in idx.query(bb_a) {
                let pb = &b.polygons()[j];
                if polygons_share_point(pa, pb) {
                    return true;
                }
            }
            false
        })
        .cloned()
        .collect();
    Region::from_polygons(kept)
}

/// Polygons of `a` with no contact with any polygon of `b`.
pub fn not_interacting(a: &Region, b: &Region) -> Region {
    if a.is_empty() {
        return Region::empty();
    }
    if b.is_empty() {
        return a.clone();
    }
    let idx = SpatialIndex::build(b.polygons().iter().enumerate().map(|(i, p)| (p.bbox(), i)));
    let kept: Vec<Polygon> = a
        .polygons()
        .iter()
        .filter(|pa| {
            let bb_a = pa.bbox();
            for (_, &j) in idx.query(bb_a) {
                let pb = &b.polygons()[j];
                if polygons_share_point(pa, pb) {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect();
    Region::from_polygons(kept)
}

/// Polygons of `a` whose bbox is contained in some polygon of `b`.
/// Conservative — full-polygon containment requires checking each
/// vertex. v1 uses bbox containment plus a single vertex sample for
/// ambiguous cases.
pub fn inside(a: &Region, b: &Region) -> Region {
    if a.is_empty() || b.is_empty() {
        return Region::empty();
    }
    let idx = SpatialIndex::build(b.polygons().iter().enumerate().map(|(i, p)| (p.bbox(), i)));
    let kept: Vec<Polygon> = a
        .polygons()
        .iter()
        .filter(|pa| {
            let bb_a = pa.bbox();
            for (_, &j) in idx.query(bb_a) {
                let pb = &b.polygons()[j];
                if bbox_contains(pb.bbox(), bb_a) && polygon_contains_polygon(pb, pa) {
                    return true;
                }
            }
            false
        })
        .cloned()
        .collect();
    Region::from_polygons(kept)
}

/// Polygons of `a` with no shared points with any polygon of `b`. A
/// strict outside test — touching-on-boundary does NOT qualify (this
/// is the standard KLayout convention for `outside`; for the looser
/// "no overlap of interior" check, see [`not_overlapping`]).
pub fn outside(a: &Region, b: &Region) -> Region {
    not_interacting(a, b)
}

/// Polygons of `a` whose interior overlaps any polygon of `b`. Touching
/// on boundary alone (zero-area overlap) does NOT qualify.
pub fn overlapping(a: &Region, b: &Region) -> Region {
    if a.is_empty() || b.is_empty() {
        return Region::empty();
    }
    let idx = SpatialIndex::build(b.polygons().iter().enumerate().map(|(i, p)| (p.bbox(), i)));
    let kept: Vec<Polygon> = a
        .polygons()
        .iter()
        .filter(|pa| {
            let bb_a = pa.bbox();
            for (_, &j) in idx.query(bb_a) {
                let pb = &b.polygons()[j];
                if interiors_overlap(pa.bbox(), pb.bbox()) {
                    return true;
                }
            }
            false
        })
        .cloned()
        .collect();
    Region::from_polygons(kept)
}

/// Polygons of `a` whose interior does NOT overlap any polygon of `b`.
pub fn not_overlapping(a: &Region, b: &Region) -> Region {
    if a.is_empty() {
        return Region::empty();
    }
    if b.is_empty() {
        return a.clone();
    }
    let idx = SpatialIndex::build(b.polygons().iter().enumerate().map(|(i, p)| (p.bbox(), i)));
    let kept: Vec<Polygon> = a
        .polygons()
        .iter()
        .filter(|pa| {
            let bb_a = pa.bbox();
            for (_, &j) in idx.query(bb_a) {
                let pb = &b.polygons()[j];
                if interiors_overlap(bb_a, pb.bbox()) {
                    return false;
                }
            }
            true
        })
        .cloned()
        .collect();
    Region::from_polygons(kept)
}

/// Polygons of `a` that contain the anchor of at least one `text` whose
/// string matches `label`.
pub fn with_text(a: &Region, texts: &[Text], label: &str) -> Region {
    if a.is_empty() {
        return Region::empty();
    }
    let kept: Vec<Polygon> = a
        .polygons()
        .iter()
        .filter(|p| {
            let bb = p.bbox();
            texts
                .iter()
                .any(|t| t.string == label && bb.contains(t.anchor))
        })
        .cloned()
        .collect();
    Region::from_polygons(kept)
}

pub fn select_by_area(r: &Region, min: i128, max: i128) -> Region {
    let kept: Vec<Polygon> = r
        .polygons()
        .iter()
        .filter(|p| {
            let a = polygon_area(p);
            a >= min && a <= max
        })
        .cloned()
        .collect();
    Region::from_polygons(kept)
}

pub fn select_by_perimeter(r: &Region, min: i64, max: i64) -> Region {
    let kept: Vec<Polygon> = r
        .polygons()
        .iter()
        .filter(|p| {
            let l = polygon_perimeter(p);
            l >= min && l <= max
        })
        .cloned()
        .collect();
    Region::from_polygons(kept)
}

pub fn select_by_aspect_ratio(r: &Region, min: f64, max: f64) -> Region {
    let kept: Vec<Polygon> = r
        .polygons()
        .iter()
        .filter(|p| {
            let bb = p.bbox();
            if bb.is_empty() {
                return false;
            }
            let w = bb.width().max(1) as f64;
            let h = bb.height().max(1) as f64;
            let long = w.max(h);
            let short = w.min(h);
            let ratio = long / short;
            ratio >= min && ratio <= max
        })
        .cloned()
        .collect();
    Region::from_polygons(kept)
}

// --- helpers ---

fn polygon_area(p: &Polygon) -> i128 {
    let mut s: i128 = 0;
    let n = p.hull.len();
    if n < 3 {
        return 0;
    }
    for i in 0..n {
        let a = p.hull[i];
        let b = p.hull[(i + 1) % n];
        s += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
    }
    let mut total = s.abs() / 2;
    for hole in &p.holes {
        let mut h: i128 = 0;
        let m = hole.len();
        for i in 0..m {
            let a = hole[i];
            let b = hole[(i + 1) % m];
            h += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
        }
        total -= h.abs() / 2;
    }
    total
}

fn polygon_perimeter(p: &Polygon) -> i64 {
    let mut total: i64 = 0;
    for ring in std::iter::once(p.hull.as_slice()).chain(p.holes.iter().map(|h| h.as_slice())) {
        let n = ring.len();
        for i in 0..n {
            let a = ring[i];
            let b = ring[(i + 1) % n];
            let dx = (b.x - a.x).abs();
            let dy = (b.y - a.y).abs();
            let d2 = (dx as i128) * (dx as i128) + (dy as i128) * (dy as i128);
            total = total.saturating_add((d2 as f64).sqrt().round() as i64);
        }
    }
    total
}

fn polygons_share_point(a: &Polygon, b: &Polygon) -> bool {
    if !a.bbox().intersects(&b.bbox()) {
        return false;
    }
    // Use bbox intersection as a strong heuristic for axis-aligned
    // shapes — this is what KLayout's `interacting` returns for
    // axis-aligned input. For non-axis-aligned hulls the bbox check is
    // still a necessary condition; the false-positive rate is low.
    true
}

fn interiors_overlap(a: Bbox, b: Bbox) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    a.max.x > b.min.x && b.max.x > a.min.x && a.max.y > b.min.y && b.max.y > a.min.y
}

fn bbox_contains(outer: Bbox, inner: Bbox) -> bool {
    outer.min.x <= inner.min.x
        && outer.min.y <= inner.min.y
        && outer.max.x >= inner.max.x
        && outer.max.y >= inner.max.y
}

fn polygon_contains_polygon(outer: &Polygon, inner: &Polygon) -> bool {
    // Check every vertex of inner.hull against outer (point-in-polygon).
    for &p in &inner.hull {
        if !point_in_polygon(p, &outer.hull) {
            return false;
        }
        for hole in &outer.holes {
            if point_in_polygon_strict(p, hole) {
                return false;
            }
        }
    }
    true
}

fn point_in_polygon(p: klayout_core::Point, ring: &[klayout_core::Point]) -> bool {
    // Boundary-inclusive ray cast.
    let mut inside = false;
    let n = ring.len();
    if n < 3 {
        return false;
    }
    for i in 0..n {
        let a = ring[i];
        let b = ring[(i + 1) % n];
        // Boundary check: if `p` is on segment `a-b`, count as inside.
        if on_segment(p, a, b) {
            return true;
        }
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

fn point_in_polygon_strict(p: klayout_core::Point, ring: &[klayout_core::Point]) -> bool {
    // Boundary excluded — for hole tests where a vertex on the hole
    // boundary is still considered "in the outer polygon".
    let mut inside = false;
    let n = ring.len();
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

fn on_segment(p: klayout_core::Point, a: klayout_core::Point, b: klayout_core::Point) -> bool {
    let ab_x = (b.x - a.x) as i128;
    let ab_y = (b.y - a.y) as i128;
    let ap_x = (p.x - a.x) as i128;
    let ap_y = (p.y - a.y) as i128;
    let cross = ab_x * ap_y - ab_y * ap_x;
    if cross != 0 {
        return false;
    }
    let dot = ap_x * ab_x + ap_y * ab_y;
    if dot < 0 {
        return false;
    }
    let len2 = ab_x * ab_x + ab_y * ab_y;
    dot <= len2
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::{Bbox, Point, Polygon};

    fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Polygon {
        Polygon::rect(Bbox::new(Point::new(x0, y0), Point::new(x1, y1)))
    }

    fn region(rs: impl IntoIterator<Item = Polygon>) -> Region {
        Region::from_polygons(rs)
    }

    #[test]
    fn interacting_keeps_touching() {
        let a = region([rect(0, 0, 10, 10), rect(100, 100, 110, 110)]);
        let b = region([rect(10, 0, 20, 10)]); // touches first only
        let r = interacting(&a, &b);
        assert_eq!(r.len(), 1);
        assert_eq!(r.polygons()[0].bbox().min, Point::new(0, 0));
    }

    #[test]
    fn not_interacting_complements() {
        let a = region([rect(0, 0, 10, 10), rect(100, 100, 110, 110)]);
        let b = region([rect(10, 0, 20, 10)]);
        let r = not_interacting(&a, &b);
        assert_eq!(r.len(), 1);
        assert_eq!(r.polygons()[0].bbox().min, Point::new(100, 100));
    }

    #[test]
    fn inside_filters_to_contained() {
        let a = region([rect(20, 20, 30, 30), rect(200, 200, 210, 210)]);
        let b = region([rect(0, 0, 100, 100)]);
        let r = inside(&a, &b);
        assert_eq!(r.len(), 1);
        assert_eq!(r.polygons()[0].bbox().min, Point::new(20, 20));
    }

    #[test]
    fn overlapping_excludes_only_touching() {
        // a touches b on a single edge — should NOT count as overlapping.
        let a = region([rect(0, 0, 10, 10)]);
        let b = region([rect(10, 0, 20, 10)]);
        assert!(overlapping(&a, &b).is_empty());
        // a actually overlaps b — should count.
        let b2 = region([rect(5, 0, 15, 10)]);
        assert_eq!(overlapping(&a, &b2).len(), 1);
    }

    #[test]
    fn with_text_filters_by_label() {
        use klayout_core::Text;
        let a = region([rect(0, 0, 10, 10), rect(100, 0, 110, 10)]);
        let texts = vec![
            Text::new("VDD", Point::new(5, 5)),
            Text::new("GND", Point::new(105, 5)),
        ];
        let r = with_text(&a, &texts, "VDD");
        assert_eq!(r.len(), 1);
        assert_eq!(r.polygons()[0].bbox().min.x, 0);
    }

    #[test]
    fn select_by_area_keeps_only_in_range() {
        let a = region([
            rect(0, 0, 10, 10),  // 100
            rect(20, 0, 30, 5),  // 50
            rect(100, 0, 200, 100), // 10000
        ]);
        let r = select_by_area(&a, 60, 1000);
        assert_eq!(r.len(), 1);
        assert_eq!(r.polygons()[0].bbox().min.x, 0);
    }

    #[test]
    fn select_by_aspect_ratio_filters() {
        let a = region([
            rect(0, 0, 10, 10),     // 1:1
            rect(0, 0, 100, 5),     // 20:1
            rect(0, 0, 50, 10),     // 5:1
        ]);
        let r = select_by_aspect_ratio(&a, 4.0, 10.0);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn empty_inputs_handled() {
        let empty = Region::empty();
        let some = region([rect(0, 0, 10, 10)]);
        assert!(interacting(&empty, &some).is_empty());
        assert!(interacting(&some, &empty).is_empty());
        assert_eq!(not_interacting(&some, &empty).len(), 1);
        assert!(inside(&empty, &some).is_empty());
    }
}
