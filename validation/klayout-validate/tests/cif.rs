//! CIF parser parity vs `klayout.db` — strict full-dump equality.
//!
//! Cell names match KLayout's `C<num>` convention, layer numbering
//! follows KLayout's `L<N>` → GDS (N, 0) mapping. Regenerate corpus
//! with `python validation/oracle.py cif`.

use klayout_io::read_cif_path;
use klayout_validate::{canonical_dump, corpus_path};
use serde_json::Value;

const FIXTURES: &[&str] = &[
    "single_box",
    "two_layers",
    "polygon_shape",
    "multi_layer_polys",
    "hierarchy",
    "rot_mirror",
];

#[test]
fn cif_parser_matches_klayout() {
    for name in FIXTURES {
        let cif_path = corpus_path(&format!("cif/{name}.cif"));
        let json_path = corpus_path(&format!("cif/{name}.json"));
        if !cif_path.exists() || !json_path.exists() {
            eprintln!("skipping cif::{name}: corpus missing");
            return;
        }
        let lib = read_cif_path(&cif_path)
            .unwrap_or_else(|e| panic!("read {} failed: {e}", cif_path.display()));
        let got = canonical_dump(&lib);
        let exp_bytes = std::fs::read(&json_path).expect("read json");
        let expected: Value = serde_json::from_slice(&exp_bytes).expect("parse json");
        if got != expected {
            panic!(
                "cif::{name} mismatch\n--- got ---\n{}\n\n--- expected ---\n{}\n",
                serde_json::to_string_pretty(&got).unwrap(),
                serde_json::to_string_pretty(&expected).unwrap(),
            );
        }
    }
}
