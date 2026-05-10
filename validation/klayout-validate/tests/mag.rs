//! MAG (Magic) parser parity vs `klayout.db` — strict full-dump
//! equality. Cell name comes from the file's basename (KLayout
//! convention); named layers use sentinel gds=(-1,-1); rect shapes
//! are emitted as boxes by both readers (KLayout v0.30.8+).

use klayout_io::read_mag_path;
use klayout_validate::{canonical_dump, corpus_path};
use serde_json::Value;

const FIXTURES: &[&str] = &["single_box", "multi_layer", "many_rects"];

#[test]
fn mag_parser_matches_klayout() {
    for name in FIXTURES {
        let mag_path = corpus_path(&format!("mag/{name}.mag"));
        let json_path = corpus_path(&format!("mag/{name}.json"));
        if !mag_path.exists() || !json_path.exists() {
            eprintln!("skipping mag::{name}: corpus missing");
            return;
        }
        let lib = read_mag_path(&mag_path)
            .unwrap_or_else(|e| panic!("read {} failed: {e}", mag_path.display()));
        let got = canonical_dump(&lib);
        let exp_bytes = std::fs::read(&json_path).expect("read json");
        let expected: Value = serde_json::from_slice(&exp_bytes).expect("parse json");
        if got != expected {
            panic!(
                "mag::{name} mismatch\n--- got ---\n{}\n\n--- expected ---\n{}\n",
                serde_json::to_string_pretty(&got).unwrap(),
                serde_json::to_string_pretty(&expected).unwrap(),
            );
        }
    }
}
