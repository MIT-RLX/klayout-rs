// Shared types + helpers for `drc.rs` and `drc_density_grid.rs`.
// Included verbatim into each integration-test crate.

use klayout_core::{Bbox, Point, Polygon};
use klayout_drc as drc;
use klayout_geom::Region;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct DrcCorpus {
    pub klayout_version: String,
    pub cases: Vec<DrcCase>,
}

#[derive(Debug, Deserialize)]
pub struct DumpedPoly {
    pub hull: Vec<[i64; 2]>,
    pub holes: Vec<Vec<[i64; 2]>>,
}

#[derive(Debug, Deserialize)]
pub struct DrcCase {
    pub rule: String,
    pub name: String,
    pub rects_a: Vec<[i64; 4]>,
    #[serde(default)]
    pub rects_b: Vec<[i64; 4]>,
    pub min: i64,
    #[serde(default)]
    pub window: Option<[i64; 2]>,
    #[serde(default)]
    pub step: Option<[i64; 2]>,
    #[serde(default)]
    pub density_min: Option<f64>,
    #[serde(default)]
    pub density_max: Option<f64>,
    #[serde(default)]
    pub density_padding: Option<String>,
    #[serde(default)]
    pub boundary_rects: Option<Vec<[i64; 4]>>,
    #[serde(default)]
    pub tile_origin: Option<[i64; 2]>,
    #[serde(default)]
    pub tile_count: Option<[i64; 2]>,
    #[serde(default)]
    pub evaluate_singleton_tiles: Option<bool>,
    #[serde(default)]
    pub density_output: Option<String>,
    pub result: Vec<DumpedPoly>,
}

pub fn region_from_rects(rects: &[[i64; 4]]) -> Region {
    Region::from_polygons(rects.iter().map(|[x0, y0, x1, y1]| {
        Polygon::rect(Bbox::new(Point::new(*x0, *y0), Point::new(*x1, *y1)))
    }))
}

pub fn canonicalize_ring(pts: &[Point]) -> Vec<[i64; 2]> {
    if pts.is_empty() {
        return Vec::new();
    }
    let n = pts.len();
    let start = (0..n).min_by_key(|&i| (pts[i].y, pts[i].x)).unwrap();
    (0..n)
        .map(|k| {
            let p = pts[(start + k) % n];
            [p.x, p.y]
        })
        .collect()
}

pub fn canonicalize_region(r: &Region) -> Vec<DumpedPoly> {
    let mut out: Vec<DumpedPoly> = r
        .polygons()
        .iter()
        .map(|p| {
            let hull = canonicalize_ring(&p.hull);
            let mut holes: Vec<Vec<[i64; 2]>> =
                p.holes.iter().map(|h| canonicalize_ring(h)).collect();
            holes.sort_by_key(|h| {
                if h.is_empty() {
                    (0, 0, 0)
                } else {
                    (h[0][0], h[0][1], h.len() as i64)
                }
            });
            DumpedPoly { hull, holes }
        })
        .collect();
    out.sort_by_key(|d| {
        if d.hull.is_empty() {
            (0, 0, 0)
        } else {
            (d.hull[0][0], d.hull[0][1], d.hull.len() as i64)
        }
    });
    out
}

pub fn assert_region_eq(case_label: &str, got: &Region, expected_polys: &[DumpedPoly]) {
    let got_canon = canonicalize_region(got);
    let mut exp_canon: Vec<DumpedPoly> = expected_polys
        .iter()
        .map(|d| DumpedPoly {
            hull: d.hull.clone(),
            holes: {
                let mut h = d.holes.clone();
                h.sort_by_key(|hole| {
                    if hole.is_empty() {
                        (0, 0, 0)
                    } else {
                        (hole[0][0], hole[0][1], hole.len() as i64)
                    }
                });
                h
            },
        })
        .collect();
    exp_canon.sort_by_key(|d| {
        if d.hull.is_empty() {
            (0, 0, 0)
        } else {
            (d.hull[0][0], d.hull[0][1], d.hull.len() as i64)
        }
    });
    if got_canon.len() != exp_canon.len()
        || got_canon
            .iter()
            .zip(exp_canon.iter())
            .any(|(g, e)| g.hull != e.hull || g.holes != e.holes)
    {
        panic!(
            "{case_label} polygon mismatch\n  got: {:#?}\n  expected: {:#?}",
            got_canon, exp_canon
        );
    }
}

pub fn apply_drc_case(case: &DrcCase, a: &Region, b: &Region) -> Region {
    match case.rule.as_str() {
        "width" => drc::width(a, case.min),
        "space" => drc::space(a, case.min),
        "separation" => drc::separation(a, b, case.min),
        "enclosing" => drc::enclosing(a, b, case.min),
        "overlap" => drc::overlap(a, b, case.min),
        "density_window" => {
            let w = case.window.expect("density_window requires window");
            let s = case.step.expect("density_window requires step");
            let dmin = case.density_min.expect("density_window requires density_min");
            let dmax = case.density_max.expect("density_window requires density_max");
            let mut cfg = drc::DensityWindowConfig::default();
            if let Some(ref p) = case.density_padding {
                cfg.padding = match p.as_str() {
                    "zero" => drc::DensityPadding::Zero,
                    "ignore" => drc::DensityPadding::Ignore,
                    x => panic!("unknown density_padding {x:?}"),
                };
            }
            if let Some(ref br) = case.boundary_rects {
                cfg.boundary = Some(region_from_rects(br));
            }
            if let Some(o) = case.tile_origin {
                cfg.tile_origin = Some((o[0], o[1]));
            }
            if let Some(tc) = case.tile_count {
                cfg.tile_count = Some((tc[0].max(0) as usize, tc[1].max(0) as usize));
            }
            if let Some(v) = case.evaluate_singleton_tiles {
                cfg.evaluate_singleton_tiles = v;
            }
            if let Some(ref o) = case.density_output {
                cfg.output = match o.as_str() {
                    "outside" => drc::DensityWindowOutput::OutsideBand,
                    "inside" => drc::DensityWindowOutput::InsideBand,
                    x => panic!("unknown density_output {x:?}"),
                };
            }
            drc::density_window_with_config(a, (w[0], w[1]), (s[0], s[1]), dmin, dmax, &cfg)
        }
        other => panic!("unknown rule '{other}'"),
    }
}
