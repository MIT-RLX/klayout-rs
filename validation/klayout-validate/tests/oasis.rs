//! OASIS reader parity: KLayout writes a `.oas`, our reader reads it,
//! canonical dumps must match.
//!
//! Mirrors the GDS suite. KLayout's OASIS writer + reader are the
//! reference; we verify our reader produces the same canonical view.

use klayout_io::read_oasis_path;
use klayout_validate::{canonical_dump, corpus_path};
use serde_json::Value;

fn assert_match(case: &str) {
    let oas = corpus_path(&format!("oasis/{case}.oas"));
    let json_path = corpus_path(&format!("oasis/{case}.json"));
    if !oas.exists() {
        eprintln!(
            "skipping oasis::{case}: corpus missing at {}.\n\
             Regenerate with: python validation/oracle.py oasis",
            oas.display()
        );
        return;
    }
    let lib = read_oasis_path(&oas)
        .unwrap_or_else(|e| panic!("read {} failed: {e}", oas.display()));
    let got = canonical_dump(&lib);
    let expected_bytes = std::fs::read(&json_path)
        .unwrap_or_else(|e| panic!("read {} failed: {e}", json_path.display()));
    let expected: Value = serde_json::from_slice(&expected_bytes).unwrap();
    if got != expected {
        let got_pretty = serde_json::to_string_pretty(&got).unwrap();
        let exp_pretty = serde_json::to_string_pretty(&expected).unwrap();
        panic!(
            "canonical mismatch for OASIS case '{case}'\n\n--- got ---\n{got_pretty}\n\n--- expected ---\n{exp_pretty}\n"
        );
    }
}

#[test]
fn single_box() { assert_match("single_box"); }
#[test]
fn hierarchy() { assert_match("hierarchy"); }
#[test]
fn polys() { assert_match("polys"); }
#[test]
fn multi_layer() { assert_match("multi_layer"); }
#[test]
fn deep_hier() { assert_match("deep_hier"); }
