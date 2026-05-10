//! Basic sanity tests for boolean ops + sizing — internally consistent
//! results, before validating against KLayout.

use klayout_core::{Bbox, Point, Polygon};
use klayout_geom::{difference, intersection, merge, size, union, xor, Region, SizeJoin};

fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Polygon {
    Polygon::rect(Bbox::new(Point::new(x0, y0), Point::new(x1, y1)))
}

fn region_of(rs: impl IntoIterator<Item = Polygon>) -> Region {
    Region::from_polygons(rs)
}

#[test]
fn union_disjoint_rects() {
    let a = region_of([rect(0, 0, 10, 10)]);
    let b = region_of([rect(20, 0, 30, 10)]);
    let r = union(&a, &b);
    assert_eq!(r.len(), 2);
    let total_bbox = r.bbox();
    assert_eq!(total_bbox.min, Point::new(0, 0));
    assert_eq!(total_bbox.max, Point::new(30, 10));
}

#[test]
fn union_overlapping_rects() {
    let a = region_of([rect(0, 0, 10, 10)]);
    let b = region_of([rect(5, 5, 15, 15)]);
    let r = union(&a, &b);
    assert_eq!(r.len(), 1, "union of two overlapping rects should be one piece");
    assert_eq!(r.bbox().min, Point::new(0, 0));
    assert_eq!(r.bbox().max, Point::new(15, 15));
}

#[test]
fn intersection_overlap() {
    let a = region_of([rect(0, 0, 10, 10)]);
    let b = region_of([rect(5, 5, 15, 15)]);
    let r = intersection(&a, &b);
    assert_eq!(r.len(), 1);
    assert_eq!(r.bbox(), Bbox::new(Point::new(5, 5), Point::new(10, 10)));
}

#[test]
fn intersection_disjoint() {
    let a = region_of([rect(0, 0, 10, 10)]);
    let b = region_of([rect(20, 0, 30, 10)]);
    let r = intersection(&a, &b);
    assert!(r.is_empty());
}

#[test]
fn difference_subtracts() {
    let a = region_of([rect(0, 0, 10, 10)]);
    let b = region_of([rect(5, 5, 15, 15)]);
    let r = difference(&a, &b);
    // a minus their overlap. Could be 1 polygon (L-shape) or 2 rects
    // depending on the engine. Either way, bbox is (0,0)-(10,10) and area
    // is 100 - 25 = 75.
    assert!(!r.is_empty());
    let total: i128 = r
        .polygons()
        .iter()
        .map(|p| {
            let mut s: i128 = 0;
            let n = p.hull.len();
            for i in 0..n {
                let a = p.hull[i];
                let b = p.hull[(i + 1) % n];
                s += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
            }
            s.abs() / 2
        })
        .sum();
    assert_eq!(total, 75);
}

#[test]
fn xor_overlap() {
    let a = region_of([rect(0, 0, 10, 10)]);
    let b = region_of([rect(5, 5, 15, 15)]);
    let r = xor(&a, &b);
    let total: i128 = r
        .polygons()
        .iter()
        .map(|p| {
            let mut s: i128 = 0;
            let n = p.hull.len();
            for i in 0..n {
                let a = p.hull[i];
                let b = p.hull[(i + 1) % n];
                s += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
            }
            s.abs() / 2
        })
        .sum();
    // (a area + b area - 2 * overlap) = 100 + 100 - 2*25 = 150
    assert_eq!(total, 150);
}

#[test]
fn size_grow_rect() {
    let a = region_of([rect(0, 0, 10, 10)]);
    let r = size(&a, 5, SizeJoin::Miter);
    assert_eq!(r.len(), 1);
    assert_eq!(r.bbox(), Bbox::new(Point::new(-5, -5), Point::new(15, 15)));
}

#[test]
fn size_shrink_rect() {
    let a = region_of([rect(0, 0, 10, 10)]);
    let r = size(&a, -2, SizeJoin::Miter);
    assert_eq!(r.len(), 1);
    assert_eq!(r.bbox(), Bbox::new(Point::new(2, 2), Point::new(8, 8)));
}

#[test]
fn xor_two_holed_polygons_with_different_hole_winding() {
    // Same shape (square with square hole), but the holes are passed in
    // with different starting vertices / windings. Polygon::add_hole
    // normalizes both to canonical CCW form so XOR sees identical regions.
    let mut a = Polygon::from_hull([
        Point::new(0, 0),
        Point::new(100, 0),
        Point::new(100, 100),
        Point::new(0, 100),
    ]);
    a.add_hole([
        Point::new(20, 20),
        Point::new(20, 50),
        Point::new(50, 50),
        Point::new(50, 20),
    ]);

    let mut b = Polygon::from_hull([
        Point::new(0, 0),
        Point::new(100, 0),
        Point::new(100, 100),
        Point::new(0, 100),
    ]);
    b.add_hole([
        Point::new(20, 20),
        Point::new(50, 20),
        Point::new(50, 50),
        Point::new(20, 50),
    ]);

    let r1 = Region::from_polygons([a]);
    let r2 = Region::from_polygons([b]);
    let d = merge(&xor(&r1, &r2));
    assert!(
        d.is_empty(),
        "XOR of identical-shape regions with hole-vertex differences should be empty, got: {:?}",
        d.polygons()
    );
}

#[test]
fn xor_with_holed_polygon_is_empty() {
    // A polygon with a hole, XOR'd with itself, should produce empty.
    // This stresses our hole orientation convention through i_overlay.
    let mut p = Polygon::from_hull([
        Point::new(0, 0),
        Point::new(100, 0),
        Point::new(100, 100),
        Point::new(0, 100),
    ]);
    p.add_hole([
        Point::new(20, 20),
        Point::new(20, 50),
        Point::new(50, 50),
        Point::new(50, 20),
    ]);
    let r1 = Region::from_polygons([p.clone()]);
    let r2 = Region::from_polygons([p]);
    let d = merge(&xor(&r1, &r2));
    assert!(
        d.is_empty(),
        "xor of identical holed regions should be empty, got polys: {:?}",
        d.polygons()
    );
}

#[test]
fn difference_produces_hole() {
    // Bigger square minus a smaller square inside it should produce one
    // polygon with a hole.
    let a = Region::from_polygons([Polygon::rect(Bbox::new(
        Point::new(0, 0),
        Point::new(100, 100),
    ))]);
    let b = Region::from_polygons([Polygon::rect(Bbox::new(
        Point::new(20, 20),
        Point::new(50, 50),
    ))]);
    let r = difference(&a, &b);
    assert_eq!(r.len(), 1);
    let p = &r.polygons()[0];
    assert_eq!(p.hull.len(), 4);
    assert_eq!(p.holes.len(), 1);
    assert_eq!(p.holes[0].len(), 4);
}

#[test]
fn size_zero_is_identity() {
    let a = region_of([rect(0, 0, 10, 10), rect(20, 20, 30, 30)]);
    let r = size(&a, 0, SizeJoin::Miter);
    assert_eq!(r.len(), 2);
    assert_eq!(r.bbox(), a.bbox());
}
