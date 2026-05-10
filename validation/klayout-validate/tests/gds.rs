//! GDS reader parity: KLayout writes a GDS, our reader reads it,
//! canonical dumps must match.

use klayout_io::read_gds_path;
use klayout_validate::{canonical_dump, corpus_path};
use serde_json::Value;

fn assert_match(case: &str) {
    let gds = corpus_path(&format!("gds/{case}.gds"));
    let json_path = corpus_path(&format!("gds/{case}.json"));

    let lib = read_gds_path(&gds)
        .unwrap_or_else(|e| panic!("read {} failed: {e}", gds.display()));
    let got = canonical_dump(&lib);

    let expected_bytes = std::fs::read(&json_path)
        .unwrap_or_else(|e| panic!("read {} failed: {e}", json_path.display()));
    let expected: Value = serde_json::from_slice(&expected_bytes).unwrap();

    if got != expected {
        let got_pretty = serde_json::to_string_pretty(&got).unwrap();
        let exp_pretty = serde_json::to_string_pretty(&expected).unwrap();
        panic!(
            "canonical mismatch for case '{case}'\n\n--- got ---\n{got_pretty}\n\n--- expected ---\n{exp_pretty}\n"
        );
    }
}

#[test]
fn single_box() { assert_match("single_box"); }
#[test]
fn mixed() { assert_match("mixed"); }
#[test]
fn hierarchy() { assert_match("hierarchy"); }
#[test]
fn aref() { assert_match("aref"); }
#[test]
fn eight_orientations() { assert_match("eight_orientations"); }
#[test]
fn path_caps() { assert_match("path_caps"); }
#[test]
fn polygons() { assert_match("polygons"); }
#[test]
fn multilayer() { assert_match("multilayer"); }
#[test]
fn multilevel() { assert_match("multilevel"); }
#[test]
fn empty_and_refonly() { assert_match("empty_and_refonly"); }
#[test]
fn aref_rotated() { assert_match("aref_rotated"); }
#[test]
fn aref_mirrored() { assert_match("aref_mirrored"); }
#[test]
fn texts() { assert_match("texts"); }
#[test]
fn mixed_shapes_and_insts() { assert_match("mixed_shapes_and_insts"); }
