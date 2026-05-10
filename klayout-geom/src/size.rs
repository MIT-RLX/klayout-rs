//! Sizing (offset / Minkowski sum with a disk-equivalent).
//!
//! Positive `delta` grows the region, negative shrinks. `Miter` joins
//! produce sharp corners (matching KLayout's default `db.Region.size`).

use crate::region::{polygon_from_int_contour, Region};
use i_overlay::mesh::outline::offset::OutlineOffset;
use i_overlay::mesh::style::{LineJoin, OutlineStyle};
use klayout_core::Polygon;
use std::f64::consts::PI;

#[derive(Copy, Clone, Debug)]
pub enum SizeJoin {
    /// Sharp corners up to a miter ratio. Matches KLayout default.
    Miter,
    /// Rounded outer corners.
    Round,
    /// Beveled corners.
    Bevel,
}

pub fn size(r: &Region, delta: i64, join: SizeJoin) -> Region {
    if delta == 0 || r.is_empty() {
        return r.clone();
    }
    let shapes: Vec<Vec<Vec<[f64; 2]>>> = r
        .polygons
        .iter()
        .map(polygon_to_floats)
        .collect();
    let style = OutlineStyle::<f64> {
        outer_offset: delta as f64,
        inner_offset: delta as f64,
        join: match join {
            SizeJoin::Miter => LineJoin::Miter(0.5 * PI),
            SizeJoin::Round => LineJoin::Round(0.05 * PI),
            SizeJoin::Bevel => LineJoin::Bevel,
        },
    };
    let result = shapes.outline(&style);

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
        polys.push(poly);
    }

    let mut out = Region::empty();
    out.set_polygons(polys);
    out
}

fn polygon_to_floats(p: &Polygon) -> Vec<Vec<[f64; 2]>> {
    // i_overlay expects CCW hull / CW holes. Our canonical form is the
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
