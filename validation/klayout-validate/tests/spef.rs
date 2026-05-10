//! SPEF parser parity vs the bundled `spef_dump.py` oracle (independent
//! pure-Python implementation, runs inside `klayout-rs-oracle:latest`).
//!
//! Regenerate the corpus with `python validation/oracle_external.py spef`.

use klayout_connect::spef::read_spef;
use klayout_validate::corpus_path;
use serde_json::{json, Value};

const FIXTURES: &[&str] = &["two_net", "with_coupling", "many_nets", "multi_coupling"];

fn dump_to_json(spef_path: &std::path::Path) -> Value {
    let text = std::fs::read_to_string(spef_path)
        .unwrap_or_else(|e| panic!("read {} failed: {e}", spef_path.display()));
    let (design, nets) = read_spef(&text)
        .unwrap_or_else(|e| panic!("parse {} failed: {e}", spef_path.display()));
    let mut nets_json: Vec<Value> = nets
        .iter()
        .map(|n| {
            let mut conns: Vec<String> = n.connections.iter().map(|s| s.to_string()).collect();
            conns.sort();
            let mut coups: Vec<(String, f64)> =
                n.coupling.iter().map(|(o, c)| (o.to_string(), *c)).collect();
            coups.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.partial_cmp(&b.1).unwrap()));
            json!({
                "name": n.name.as_str(),
                "total_cap": n.total_cap,
                "connections": conns,
                "coupling": coups.iter().map(|(o, c)| json!([o, c])).collect::<Vec<_>>(),
                "resistance": n.resistance,
            })
        })
        .collect();
    nets_json.sort_by(|a, b| a["name"].as_str().unwrap().cmp(b["name"].as_str().unwrap()));
    json!({ "design": design, "nets": nets_json })
}

#[test]
fn spef_parser_matches_oracle() {
    for name in FIXTURES {
        let spef_path = corpus_path(&format!("spef/{name}.spef"));
        let json_path = corpus_path(&format!("spef/{name}.json"));
        if !spef_path.exists() || !json_path.exists() {
            eprintln!(
                "skipping spef::{name}: corpus missing.\n\
                 Regenerate with: python validation/oracle_external.py spef"
            );
            return;
        }
        let got = dump_to_json(&spef_path);
        let exp_bytes = std::fs::read(&json_path).expect("read json");
        let expected: Value = serde_json::from_slice(&exp_bytes).expect("parse json");
        if got != expected {
            let g = serde_json::to_string_pretty(&got).unwrap();
            let e = serde_json::to_string_pretty(&expected).unwrap();
            panic!("spef::{name} mismatch\n--- got ---\n{g}\n\n--- expected (oracle) ---\n{e}\n");
        }
    }
}
