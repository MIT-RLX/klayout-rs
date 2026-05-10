//! Differential test: our `Trans` against KLayout's `db.Trans`.
//!
//! Each case from `corpus/trans.json` was produced by the oracle running
//! against `klayout.db`. We must produce identical results — exact equality
//! of integer outputs, no tolerance.

use klayout_core::{Point, Rot4, Trans, Vec2};
use klayout_validate::corpus_path;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct TransCorpus {
    klayout_version: String,
    apply_cases: Vec<ApplyCase>,
    compose_cases: Vec<ComposeCase>,
    inverse_cases: Vec<InverseCase>,
}

#[derive(Debug, Deserialize)]
struct ApplyCase {
    trans: [i64; 3],   // [rot_combined_0..7, dx, dy]
    point: [i64; 2],
    result: [i64; 2],
}

#[derive(Debug, Deserialize)]
struct ComposeCase {
    a: [i64; 3],
    b: [i64; 3],
    result: [i64; 3],
}

#[derive(Debug, Deserialize)]
struct InverseCase {
    trans: [i64; 3],
    result: [i64; 3],
}

/// Map KLayout's combined rot code (0..7) to our `(Rot4, mirror)`.
fn from_klayout(rot_combined: i64, dx: i64, dy: i64) -> Trans {
    let mirror = rot_combined >= 4;
    let r = rot_combined % 4;
    let rot = match r {
        0 => Rot4::R0,
        1 => Rot4::R90,
        2 => Rot4::R180,
        3 => Rot4::R270,
        _ => unreachable!(),
    };
    Trans::new(rot, mirror, Vec2::new(dx, dy))
}

fn to_klayout(t: Trans) -> [i64; 3] {
    let r = match t.rot {
        Rot4::R0 => 0,
        Rot4::R90 => 1,
        Rot4::R180 => 2,
        Rot4::R270 => 3,
    };
    let combined = if t.mirror { r + 4 } else { r };
    [combined, t.disp.x, t.disp.y]
}

fn load() -> TransCorpus {
    let path = corpus_path("trans.json");
    let bytes = std::fs::read(&path)
        .unwrap_or_else(|e| panic!("missing corpus {}: {e}\nrun: python validation/oracle.py", path.display()));
    serde_json::from_slice(&bytes).expect("invalid trans corpus JSON")
}

#[test]
fn apply_matches_klayout() {
    let c = load();
    let mut failed = 0usize;
    for (i, case) in c.apply_cases.iter().enumerate() {
        let t = from_klayout(case.trans[0], case.trans[1], case.trans[2]);
        let got = t.apply(Point::new(case.point[0], case.point[1]));
        let expected = Point::new(case.result[0], case.result[1]);
        if got != expected {
            if failed < 5 {
                eprintln!(
                    "case {i}: trans={:?} point={:?} got={got:?} expected={expected:?}",
                    case.trans, case.point
                );
            }
            failed += 1;
        }
    }
    assert_eq!(
        failed, 0,
        "{failed}/{} apply cases differ from KLayout {}",
        c.apply_cases.len(),
        c.klayout_version
    );
}

#[test]
fn compose_matches_klayout() {
    let c = load();
    let mut failed = 0usize;
    for (i, case) in c.compose_cases.iter().enumerate() {
        let a = from_klayout(case.a[0], case.a[1], case.a[2]);
        let b = from_klayout(case.b[0], case.b[1], case.b[2]);
        let got = a.compose(b);
        let expected = from_klayout(case.result[0], case.result[1], case.result[2]);
        if got != expected {
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
    assert_eq!(
        failed, 0,
        "{failed}/{} compose cases differ from KLayout",
        c.compose_cases.len()
    );
}

#[test]
fn inverse_matches_klayout() {
    let c = load();
    let mut failed = 0usize;
    for (i, case) in c.inverse_cases.iter().enumerate() {
        let t = from_klayout(case.trans[0], case.trans[1], case.trans[2]);
        let got = t.inverse();
        let expected = from_klayout(case.result[0], case.result[1], case.result[2]);
        if got != expected {
            if failed < 5 {
                eprintln!(
                    "case {i}: trans={:?} got={:?} expected={:?}",
                    case.trans,
                    to_klayout(got),
                    case.result
                );
            }
            failed += 1;
        }
    }
    assert_eq!(
        failed, 0,
        "{failed}/{} inverse cases differ from KLayout",
        c.inverse_cases.len()
    );
}
