//! Differential test: our `Region` boolean ops + sizing against `db.Region`.
//!
//! Inputs are simple axis-aligned rectangles; expected outputs are merged,
//! canonicalized polygon hulls (lowest-y/lowest-x first, clockwise winding).

use klayout_core::{Bbox, Point, Polygon};
use klayout_geom::{difference, intersection, size, union, xor, Region, SizeJoin};
use klayout_validate::corpus_path;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct RegionCorpus {
    klayout_version: String,
    boolean_cases: Vec<BooleanCase>,
    size_cases: Vec<SizeCase>,
}

#[derive(Debug, Deserialize)]
struct DumpedPoly {
    hull: Vec<[i64; 2]>,
    holes: Vec<Vec<[i64; 2]>>,
}

#[derive(Debug, Deserialize)]
struct BooleanCase {
    name: String,
    a_rects: Vec<[i64; 4]>,
    b_rects: Vec<[i64; 4]>,
    union: Vec<DumpedPoly>,
    intersect: Vec<DumpedPoly>,
    difference: Vec<DumpedPoly>,
    xor: Vec<DumpedPoly>,
}

#[derive(Debug, Deserialize)]
struct SizeCase {
    name: String,
    rects: Vec<[i64; 4]>,
    delta: i64,
    result: Vec<DumpedPoly>,
}

fn region_from_rects(rects: &[[i64; 4]]) -> Region {
    Region::from_polygons(
        rects
            .iter()
            .map(|[x0, y0, x1, y1]| {
                Polygon::rect(Bbox::new(Point::new(*x0, *y0), Point::new(*x1, *y1)))
            }),
    )
}

#[allow(dead_code)]
fn region_from_dump(polys: &[DumpedPoly]) -> Region {
    Region::from_polygons(polys.iter().map(|dp| {
        let mut p = Polygon::from_hull(dp.hull.iter().map(|pt| Point::new(pt[0], pt[1])));
        for hole in &dp.holes {
            p.add_hole(hole.iter().map(|pt| Point::new(pt[0], pt[1])));
        }
        p
    }))
}

/// Merge disjoint polygons that touch at vertex points into a single
/// polygon-with-hole — KLayout's boolean ops always produce that
/// representation, while `i_overlay` leaves them as separate components.
///
/// Handles the common case of two polygons that share exactly two
/// vertices (e.g. XOR of overlapping rectangles producing two L-shapes
/// that kiss at the diagonal corners). Splits each polygon's boundary
/// at the shared vertices into "outer" and "inner" arcs, then concats
/// outer arcs into the merged hull and inner arcs into the hole.
fn klayoutize(mut polys: Vec<Polygon>) -> Vec<Polygon> {
    loop {
        let mut merged_any = false;
        'pairs: for i in 0..polys.len() {
            for j in (i + 1)..polys.len() {
                let shared = shared_vertices(&polys[i].hull, &polys[j].hull);
                if shared.len() == 2 {
                    if let Some(new_poly) = merge_two_at_vertices(
                        &polys[i],
                        &polys[j],
                        shared[0],
                        shared[1],
                    ) {
                        let pj = polys.remove(j);
                        let _ = pj;
                        polys[i] = new_poly;
                        merged_any = true;
                        break 'pairs;
                    }
                }
            }
        }
        if !merged_any {
            return polys;
        }
    }
}

fn shared_vertices(a: &[Point], b: &[Point]) -> Vec<Point> {
    let bset: std::collections::HashSet<(i64, i64)> =
        b.iter().map(|p| (p.x, p.y)).collect();
    a.iter()
        .filter(|p| bset.contains(&(p.x, p.y)))
        .copied()
        .collect()
}

fn merge_two_at_vertices(p1: &Polygon, p2: &Polygon, v0: Point, v1: Point) -> Option<Polygon> {
    let (a_long, a_short) = split_ring(&p1.hull, v0, v1)?;
    let (b_long, b_short) = split_ring(&p2.hull, v0, v1)?;
    // The merged outer hull = the two longer arcs joined; the hole = the
    // two shorter arcs joined. Pick by arc length (vertex count).
    let outer_arc_a;
    let outer_arc_b;
    let inner_arc_a;
    let inner_arc_b;
    if a_long.len() >= a_short.len() {
        outer_arc_a = a_long;
        inner_arc_a = a_short;
    } else {
        outer_arc_a = a_short;
        inner_arc_a = a_long;
    }
    if b_long.len() >= b_short.len() {
        outer_arc_b = b_long;
        inner_arc_b = b_short;
    } else {
        outer_arc_b = b_short;
        inner_arc_b = b_long;
    }
    // Stitch outer arcs (they both go between v0 and v1 — orient so the
    // result is a single closed loop).
    let hull = stitch(&outer_arc_a, &outer_arc_b, v0, v1)?;
    let hole = stitch(&inner_arc_a, &inner_arc_b, v0, v1)?;
    let mut poly = Polygon::from_hull(hull);
    if hole.len() >= 3 {
        poly.add_hole(hole);
    }
    Some(poly)
}

/// Walk `ring` and split it into the two arcs between the two boundary
/// points `v0` and `v1`. Returns (arc1, arc2) where each arc starts
/// and ends at `v0`/`v1` and the arcs include both endpoints.
fn split_ring(ring: &[Point], v0: Point, v1: Point) -> Option<(Vec<Point>, Vec<Point>)> {
    let n = ring.len();
    let i0 = ring.iter().position(|p| *p == v0)?;
    let i1 = ring.iter().position(|p| *p == v1)?;
    let mut a = Vec::new();
    let mut idx = i0;
    loop {
        a.push(ring[idx]);
        if idx == i1 {
            break;
        }
        idx = (idx + 1) % n;
    }
    let mut b = Vec::new();
    let mut idx = i1;
    loop {
        b.push(ring[idx]);
        if idx == i0 {
            break;
        }
        idx = (idx + 1) % n;
    }
    Some((a, b))
}

/// Stitch two arcs between `v0` and `v1` into a closed loop. Orients
/// them so the loop is traversed once — picks whichever direction makes
/// the endpoints align.
fn stitch(arc_a: &[Point], arc_b: &[Point], v0: Point, v1: Point) -> Option<Vec<Point>> {
    if arc_a.len() < 2 || arc_b.len() < 2 {
        return None;
    }
    // Start at v0, go through arc_a to v1 (or v0->v1).
    let a_oriented: Vec<Point> = if arc_a[0] == v0 && *arc_a.last().unwrap() == v1 {
        arc_a.to_vec()
    } else if arc_a[0] == v1 && *arc_a.last().unwrap() == v0 {
        arc_a.iter().rev().copied().collect()
    } else {
        return None;
    };
    // Then v1 back to v0 via arc_b.
    let b_oriented: Vec<Point> = if arc_b[0] == v1 && *arc_b.last().unwrap() == v0 {
        arc_b.to_vec()
    } else if arc_b[0] == v0 && *arc_b.last().unwrap() == v1 {
        arc_b.iter().rev().copied().collect()
    } else {
        return None;
    };
    let mut out = a_oriented;
    // Drop duplicate v1 at start of b (already at end of a).
    out.extend(b_oriented.iter().skip(1).copied());
    // Drop the final duplicate v0 (we'll re-close implicitly).
    if out.last() == Some(&v0) {
        out.pop();
    }
    Some(out)
}

/// Canonicalize a ring to lowest-y/lowest-x first vertex while preserving
/// winding direction — same convention `oracle.py:canonicalize_polygon`
/// uses for KLayout's output.
fn canonicalize_ring(pts: &[Point]) -> Vec<[i64; 2]> {
    if pts.is_empty() {
        return Vec::new();
    }
    let n = pts.len();
    let start = (0..n)
        .min_by_key(|&i| (pts[i].y, pts[i].x))
        .unwrap();
    (0..n).map(|k| {
        let p = pts[(start + k) % n];
        [p.x, p.y]
    }).collect()
}

fn canonicalize_region(r: &Region) -> Vec<DumpedPoly> {
    let merged = klayoutize(r.polygons().to_vec());
    let mut out: Vec<DumpedPoly> = merged
        .iter()
        .map(|p| {
            let hull = canonicalize_ring(&p.hull);
            let mut holes: Vec<Vec<[i64; 2]>> = p
                .holes
                .iter()
                .map(|h| canonicalize_ring(h))
                .collect();
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

fn dumped_eq(a: &DumpedPoly, b: &DumpedPoly) -> bool {
    a.hull == b.hull && a.holes == b.holes
}

/// Strict vertex-equality after canonicalization. Both sides go through
/// the same lowest-y/lowest-x rotation + hole sort, so identical polygons
/// (including holes) compare byte-equal.
fn assert_region_eq(case: &str, op: &str, got: &Region, expected_polys: &[DumpedPoly]) {
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
            .any(|(g, e)| !dumped_eq(g, e))
    {
        panic!(
            "case '{case}' op '{op}' polygon mismatch\n  got: {:#?}\n  expected: {:#?}",
            got_canon, exp_canon
        );
    }
}

fn load() -> RegionCorpus {
    let path = corpus_path("region.json");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("missing {}: {e}\nrun: python validation/oracle.py region", path.display()));
    serde_json::from_slice(&bytes).expect("invalid region corpus JSON")
}

#[test]
fn boolean_ops_match_klayout() {
    let c = load();
    for case in &c.boolean_cases {
        let a = region_from_rects(&case.a_rects);
        let b = region_from_rects(&case.b_rects);

        assert_region_eq(&case.name, "union", &union(&a, &b), &case.union);
        assert_region_eq(&case.name, "intersect", &intersection(&a, &b), &case.intersect);
        assert_region_eq(&case.name, "difference", &difference(&a, &b), &case.difference);
        assert_region_eq(&case.name, "xor", &xor(&a, &b), &case.xor);
    }
    eprintln!(
        "{} boolean cases × 4 ops match KLayout {}",
        c.boolean_cases.len(),
        c.klayout_version
    );
}

#[test]
fn size_matches_klayout() {
    let c = load();
    for case in &c.size_cases {
        let r = region_from_rects(&case.rects);
        let got = size(&r, case.delta, SizeJoin::Miter);
        assert_region_eq(&case.name, "size", &got, &case.result);
    }
    eprintln!("{} size cases match KLayout", c.size_cases.len());
}
