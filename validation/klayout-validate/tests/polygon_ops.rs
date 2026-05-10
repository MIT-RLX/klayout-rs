//! Polygon scalar properties — `area`, `perimeter`, `bbox` — versus
//! `db.Polygon`. Inputs are simple closed hulls; outputs are exact
//! integer values.

use klayout_core::{Bbox, Point, Polygon};
use klayout_validate::corpus_path;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct PolygonOpsCorpus {
    cases: Vec<Case>,
}

#[derive(Debug, Deserialize)]
struct Case {
    name: String,
    hull: Vec<[i64; 2]>,
    area: i64,
    perimeter: i64,
    bbox: [i64; 4],
}

fn load() -> Option<PolygonOpsCorpus> {
    let path = corpus_path("polygon_ops.json");
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => {
            eprintln!(
                "skipping polygon_ops: corpus missing at {}.\n\
                 Regenerate with: python validation/oracle.py polygon_ops",
                path.display()
            );
            return None;
        }
    };
    Some(serde_json::from_slice(&bytes).expect("invalid polygon_ops corpus"))
}

fn polygon_area(p: &Polygon) -> i64 {
    let mut s: i128 = 0;
    let n = p.hull.len();
    for i in 0..n {
        let a = p.hull[i];
        let b = p.hull[(i + 1) % n];
        s += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
    }
    (s.abs() / 2) as i64
}

fn polygon_perimeter(p: &Polygon) -> i64 {
    let mut total: i64 = 0;
    let n = p.hull.len();
    for i in 0..n {
        let a = p.hull[i];
        let b = p.hull[(i + 1) % n];
        let dx = (b.x - a.x).abs();
        let dy = (b.y - a.y).abs();
        // Manhattan for axis-aligned; Euclidean fall-back for diagonals.
        if a.x == b.x || a.y == b.y {
            total += dx + dy;
        } else {
            let d2 = (dx as i128) * (dx as i128) + (dy as i128) * (dy as i128);
            total += (d2 as f64).sqrt().round() as i64;
        }
    }
    total
}

#[test]
fn area_matches_klayout() {
    let Some(c) = load() else { return };
    let mut mismatches: Vec<String> = Vec::new();
    for case in &c.cases {
        let p = Polygon::from_hull(case.hull.iter().map(|pt| Point::new(pt[0], pt[1])));
        let got = polygon_area(&p);
        if got != case.area {
            mismatches.push(format!(
                "{}: expected area {}, got {}",
                case.name, case.area, got
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    eprintln!("{} cases × area match", c.cases.len());
}

#[test]
fn perimeter_matches_klayout() {
    let Some(c) = load() else { return };
    let mut mismatches: Vec<String> = Vec::new();
    for case in &c.cases {
        let p = Polygon::from_hull(case.hull.iter().map(|pt| Point::new(pt[0], pt[1])));
        let got = polygon_perimeter(&p);
        // Allow ±1 DBU on diagonal edges due to f64 rounding.
        if (got - case.perimeter).abs() > 1 {
            mismatches.push(format!(
                "{}: expected perimeter {}, got {}",
                case.name, case.perimeter, got
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "{} mismatches:\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    eprintln!("{} cases × perimeter match", c.cases.len());
}

#[test]
fn bbox_matches_klayout() {
    let Some(c) = load() else { return };
    for case in &c.cases {
        let p = Polygon::from_hull(case.hull.iter().map(|pt| Point::new(pt[0], pt[1])));
        let got = p.bbox();
        let expected = Bbox::new(
            Point::new(case.bbox[0], case.bbox[1]),
            Point::new(case.bbox[2], case.bbox[3]),
        );
        assert_eq!(got, expected, "case {}: bbox mismatch", case.name);
    }
    eprintln!("{} cases × bbox match", c.cases.len());
}
