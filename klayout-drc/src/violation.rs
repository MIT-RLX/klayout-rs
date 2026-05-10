//! Violation reporting helpers.
//!
//! Rule primitives return a `Region`; this module converts a `Region` into
//! a sortable `Vec<Violation>` for reporting / serialization.

use klayout_core::Bbox;
use klayout_geom::Region;
use smol_str::SmolStr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub rule: SmolStr,
    pub bbox: Bbox,
    /// Twice the absolute signed area, for consistent integer comparison.
    /// Real area = `area2 / 2`.
    pub area2: i128,
}

pub fn violations_from_region(rule: impl Into<SmolStr>, r: &Region) -> Vec<Violation> {
    let rule_name = rule.into();
    let mut out: Vec<Violation> = r
        .polygons()
        .iter()
        .map(|p| Violation {
            rule: rule_name.clone(),
            bbox: p.bbox(),
            area2: polygon_area2(p),
        })
        .collect();
    out.sort_by_key(|v| (v.bbox.min.x, v.bbox.min.y, v.bbox.max.x, v.bbox.max.y));
    out
}

fn polygon_area2(p: &klayout_core::Polygon) -> i128 {
    let mut s: i128 = 0;
    let n = p.hull.len();
    for i in 0..n {
        let a = p.hull[i];
        let b = p.hull[(i + 1) % n];
        s += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
    }
    s.abs()
}
