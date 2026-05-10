//! Differential test: our `Bbox` against KLayout's `db.Box`.
//!
//! `Option<[i64;4]>` ↔ `Bbox`: `None` is the empty box. KLayout's
//! `Box(0,0,0,0)` (degenerate point box) is *not* empty — width and height
//! zero but `empty()` is false. Our `Bbox::new(p, p)` matches that.

use klayout_core::{Bbox, Point, Rot4, Trans, Vec2};
use klayout_validate::corpus_path;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct BboxCorpus {
    klayout_version: String,
    union_cases: Vec<BoxOpCase>,
    intersect_cases: Vec<BoxOpCase>,
    contains_cases: Vec<ContainsCase>,
    transform_cases: Vec<TransformCase>,
    edge_cases: Vec<EdgeCase>,
}

#[derive(Debug, Deserialize)]
struct BoxOpCase {
    a: Option<[i64; 4]>,
    b: Option<[i64; 4]>,
    result: Option<[i64; 4]>,
}

#[derive(Debug, Deserialize)]
struct ContainsCase {
    #[serde(rename = "box")]
    box_: Option<[i64; 4]>,
    point: [i64; 2],
    result: bool,
}

#[derive(Debug, Deserialize)]
struct TransformCase {
    trans: [i64; 3],
    #[serde(rename = "box")]
    box_: Option<[i64; 4]>,
    result: Option<[i64; 4]>,
}

#[derive(Debug, Deserialize)]
struct EdgeCase {
    #[serde(rename = "box")]
    box_: Option<[i64; 4]>,
    empty: bool,
    // width/height for empty boxes are platform-dependent garbage in KLayout
    // (u32 wrap of negative); we don't compare those.
    #[allow(dead_code)]
    width: i64,
    #[allow(dead_code)]
    height: i64,
}

fn from_klayout(b: Option<[i64; 4]>) -> Bbox {
    match b {
        None => Bbox::EMPTY,
        Some([l, bot, r, t]) => Bbox::new(Point::new(l, bot), Point::new(r, t)),
    }
}

fn to_klayout(b: Bbox) -> Option<[i64; 4]> {
    if b.is_empty() {
        None
    } else {
        Some([b.min.x, b.min.y, b.max.x, b.max.y])
    }
}

fn from_klayout_trans(arr: [i64; 3]) -> Trans {
    let combined = arr[0];
    let mirror = combined >= 4;
    let r = combined % 4;
    let rot = match r {
        0 => Rot4::R0,
        1 => Rot4::R90,
        2 => Rot4::R180,
        3 => Rot4::R270,
        _ => unreachable!(),
    };
    Trans::new(rot, mirror, Vec2::new(arr[1], arr[2]))
}

fn load() -> BboxCorpus {
    let path = corpus_path("bbox.json");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("missing corpus {}: {e}", path.display()));
    serde_json::from_slice(&bytes).expect("invalid bbox corpus JSON")
}

#[test]
fn union_matches_klayout() {
    let c = load();
    let mut failed = 0usize;
    for (i, case) in c.union_cases.iter().enumerate() {
        let a = from_klayout(case.a);
        let b = from_klayout(case.b);
        let got = a.union(&b);
        let expected = from_klayout(case.result);
        if to_klayout(got) != to_klayout(expected) {
            if failed < 5 {
                eprintln!(
                    "case {i}: a={:?} b={:?} got={:?} expected={:?}",
                    case.a,
                    case.b,
                    to_klayout(got),
                    case.result
                );
            }
            failed += 1;
        }
    }
    assert_eq!(failed, 0, "{failed}/{} union cases differ from KLayout {}", c.union_cases.len(), c.klayout_version);
}

#[test]
fn intersect_matches_klayout() {
    let c = load();
    let mut failed = 0usize;
    for (i, case) in c.intersect_cases.iter().enumerate() {
        let a = from_klayout(case.a);
        let b = from_klayout(case.b);
        let got = a.intersection(&b);
        let expected = from_klayout(case.result);
        if to_klayout(got) != to_klayout(expected) {
            if failed < 5 {
                eprintln!(
                    "case {i}: a={:?} b={:?} got={:?} expected={:?}",
                    case.a,
                    case.b,
                    to_klayout(got),
                    case.result
                );
            }
            failed += 1;
        }
    }
    assert_eq!(failed, 0, "{failed}/{} intersect cases differ", c.intersect_cases.len());
}

#[test]
fn contains_matches_klayout() {
    let c = load();
    let mut failed = 0usize;
    for (i, case) in c.contains_cases.iter().enumerate() {
        let b = from_klayout(case.box_);
        let p = Point::new(case.point[0], case.point[1]);
        let got = b.contains(p);
        if got != case.result {
            if failed < 5 {
                eprintln!(
                    "case {i}: box={:?} point={:?} got={got} expected={}",
                    case.box_, case.point, case.result
                );
            }
            failed += 1;
        }
    }
    assert_eq!(failed, 0, "{failed}/{} contains cases differ", c.contains_cases.len());
}

#[test]
fn transform_matches_klayout() {
    let c = load();
    let mut failed = 0usize;
    for (i, case) in c.transform_cases.iter().enumerate() {
        let t = from_klayout_trans(case.trans);
        let b = from_klayout(case.box_);
        let got = t.apply_bbox(b);
        let expected = from_klayout(case.result);
        if to_klayout(got) != to_klayout(expected) {
            if failed < 5 {
                eprintln!(
                    "case {i}: trans={:?} box={:?} got={:?} expected={:?}",
                    case.trans,
                    case.box_,
                    to_klayout(got),
                    case.result
                );
            }
            failed += 1;
        }
    }
    assert_eq!(failed, 0, "{failed}/{} transform cases differ", c.transform_cases.len());
}

#[test]
fn empty_semantics_match_klayout() {
    let c = load();
    for case in &c.edge_cases {
        let b = from_klayout(case.box_);
        assert_eq!(
            b.is_empty(),
            case.empty,
            "empty mismatch for {:?}: ours={} klayout={}",
            case.box_,
            b.is_empty(),
            case.empty
        );
    }
}
