//! Differential test: our DRC primitives against `db.Region.{width,space,...}_check`.
//!
//! Comparison is by strict vertex equality after canonicalizing each
//! polygon to lowest-y/lowest-x first vertex (KLayout's convention).

use klayout_core::{Bbox, Point, Polygon};
use klayout_drc as drc;
use klayout_geom::Region;
use klayout_validate::corpus_path;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct DrcCorpus {
    klayout_version: String,
    cases: Vec<DrcCase>,
}

#[derive(Debug, Deserialize)]
struct DumpedPoly {
    hull: Vec<[i64; 2]>,
    holes: Vec<Vec<[i64; 2]>>,
}

#[derive(Debug, Deserialize)]
struct DrcCase {
    rule: String,
    name: String,
    rects_a: Vec<[i64; 4]>,
    #[serde(default)]
    rects_b: Vec<[i64; 4]>,
    min: i64,
    result: Vec<DumpedPoly>,
}

fn region_from_rects(rects: &[[i64; 4]]) -> Region {
    Region::from_polygons(rects.iter().map(|[x0, y0, x1, y1]| {
        Polygon::rect(Bbox::new(Point::new(*x0, *y0), Point::new(*x1, *y1)))
    }))
}

fn canonicalize_ring(pts: &[Point]) -> Vec<[i64; 2]> {
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

fn canonicalize_region(r: &Region) -> Vec<DumpedPoly> {
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

fn assert_region_eq(case_label: &str, got: &Region, expected_polys: &[DumpedPoly]) {
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

fn load() -> DrcCorpus {
    let path = corpus_path("drc.json");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("missing {}: {e}\nrun: python validation/oracle.py drc", path.display()));
    serde_json::from_slice(&bytes).expect("invalid drc corpus JSON")
}

#[test]
fn drc_rules_match_klayout() {
    let c = load();
    for case in &c.cases {
        let label = format!("[{}/{}]", case.rule, case.name);
        let a = region_from_rects(&case.rects_a);
        let b = region_from_rects(&case.rects_b);
        let got = match case.rule.as_str() {
            "width" => drc::width(&a, case.min),
            "space" => drc::space(&a, case.min),
            "separation" => drc::separation(&a, &b, case.min),
            "enclosing" => drc::enclosing(&a, &b, case.min),
            "overlap" => drc::overlap(&a, &b, case.min),
            other => panic!("unknown rule '{other}'"),
        };
        assert_region_eq(&label, &got, &case.result);
    }
    eprintln!("{} DRC cases match KLayout {}", c.cases.len(), c.klayout_version);
}
