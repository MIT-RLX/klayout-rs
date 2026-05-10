//! Boolean ops on `Region`s, backed by `i_overlay`.
//!
//! All ops produce merged, canonicalized output. The float bridge uses
//! `[f64; 2]` — exact for any i64 coordinate < 2^53 (chip-scale layouts
//! never come anywhere near that).

use crate::region::{polygon_from_int_contour, Region};
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::single::SingleFloatOverlay;
use klayout_core::Polygon;

pub fn union(a: &Region, b: &Region) -> Region {
    overlay(a, b, OverlayRule::Union)
}

pub fn intersection(a: &Region, b: &Region) -> Region {
    overlay(a, b, OverlayRule::Intersect)
}

pub fn difference(a: &Region, b: &Region) -> Region {
    overlay(a, b, OverlayRule::Difference)
}

pub fn xor(a: &Region, b: &Region) -> Region {
    overlay(a, b, OverlayRule::Xor)
}

/// Self-merge: collapse a region into its merged canonical form.
/// Equivalent to `union(r, &Region::empty())`.
pub fn merge(r: &Region) -> Region {
    overlay(r, &Region::empty(), OverlayRule::Union)
}

fn polygon_to_floats(p: &Polygon) -> Vec<Vec<[f64; 2]>> {
    // i_overlay convention: CCW hull, CW holes. Our canonical form is the
    // opposite (CW hull / CCW holes), so reverse on the way in.
    let mut out: Vec<Vec<[f64; 2]>> = Vec::with_capacity(1 + p.holes.len());
    out.push(
        p.hull
            .iter()
            .rev()
            .map(|pt| [pt.x as f64, pt.y as f64])
            .collect(),
    );
    for hole in &p.holes {
        out.push(
            hole.iter()
                .rev()
                .map(|pt| [pt.x as f64, pt.y as f64])
                .collect(),
        );
    }
    out
}

fn region_to_shapes(r: &Region) -> Vec<Vec<Vec<[f64; 2]>>> {
    r.polygons.iter().map(polygon_to_floats).collect()
}

fn overlay(a: &Region, b: &Region, rule: OverlayRule) -> Region {
    let subj = region_to_shapes(a);
    let clip = region_to_shapes(b);
    // i_overlay returns Vec<shapes> where each shape = Vec<contours> and
    // contour = Vec<[f64; 2]>. The first contour is the hull, the rest are
    // holes.
    let result = subj.overlay(&clip, rule, FillRule::NonZero);

    let mut polys: Vec<Polygon> = Vec::with_capacity(result.len());
    for shape in result {
        let mut iter = shape.into_iter();
        let Some(hull_f) = iter.next() else { continue };
        let hull_int: Vec<(i64, i64)> = hull_f
            .iter()
            .map(|p| (p[0].round() as i64, p[1].round() as i64))
            .collect();
        let Some(mut poly) = polygon_from_int_contour(&hull_int) else { continue };
        for hole_f in iter {
            let hole_int: Vec<(i64, i64)> = hole_f
                .iter()
                .map(|p| (p[0].round() as i64, p[1].round() as i64))
                .collect();
            if hole_int.len() >= 3 {
                poly.add_hole(
                    hole_int
                        .into_iter()
                        .map(|(x, y)| klayout_core::Point::new(x, y)),
                );
            }
        }
        // Drop degenerate (zero-area) results from edge overlap.
        if signed_area2(&poly) != 0 {
            polys.push(poly);
        }
    }

    let mut out = Region::empty();
    out.set_polygons(polys);
    out
}

fn signed_area2(p: &Polygon) -> i128 {
    let mut s: i128 = 0;
    let n = p.hull.len();
    for i in 0..n {
        let a = p.hull[i];
        let b = p.hull[(i + 1) % n];
        s += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
    }
    s
}
