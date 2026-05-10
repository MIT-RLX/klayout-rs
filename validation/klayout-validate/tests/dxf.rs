//! DXF parser parity vs `klayout.db` — strict full-dump equality.
//! Top-cell name "TOP" matches KLayout; numeric DXF layer names map
//! to GDS (N, 0); other names use sentinel gds=(-1, -1).

use klayout_io::read_dxf_path;
use klayout_validate::{canonical_dump, corpus_path};
use serde_json::Value;

const FIXTURES: &[&str] = &["rectangle", "two_polylines", "lines_polys"];

#[test]
fn dxf_parser_matches_klayout() {
    for name in FIXTURES {
        let dxf_path = corpus_path(&format!("dxf/{name}.dxf"));
        let json_path = corpus_path(&format!("dxf/{name}.json"));
        if !dxf_path.exists() || !json_path.exists() {
            eprintln!("skipping dxf::{name}: corpus missing");
            return;
        }
        let lib = read_dxf_path(&dxf_path)
            .unwrap_or_else(|e| panic!("read {} failed: {e}", dxf_path.display()));
        let got = canonical_dump(&lib);
        let exp_bytes = std::fs::read(&json_path).expect("read json");
        let expected: Value = serde_json::from_slice(&exp_bytes).expect("parse json");
        if got != expected {
            panic!(
                "dxf::{name} mismatch\n--- got ---\n{}\n\n--- expected ---\n{}\n",
                serde_json::to_string_pretty(&got).unwrap(),
                serde_json::to_string_pretty(&expected).unwrap(),
            );
        }
    }
}
