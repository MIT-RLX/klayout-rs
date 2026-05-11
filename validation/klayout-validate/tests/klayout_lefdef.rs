//! LEF/DEF **layout** parity vs KLayout's native importer (`klayout.db` read with
//! LEF/DEF options). Compares a compact geometry hash (sorted cells, `layer_base`,
//! boxes/paths/polygons, instance transforms) — not raw GDS layer numbers, which
//! differ between readers.
//!
//! Regenerate reference JSON with:
//! `python validation/oracle_klayout_lefdef.py`
//!
//! Requires the same `.def` / `tech.lef` / `cells.lef` triples as `tests/def.rs`.

use klayout_lef::{read_def_full, read_lef_full};
use klayout_validate::{corpus_path, lefdef_klayout_parity_dump};
use serde_json::Value;

const FIXTURES: &[&str] = &[
    "inv_chain",
    "mixed_cells",
    "orientations",
    "nand_chain",
    "seq_design",
    "with_routes",
];

fn build_lib() -> klayout_lef::LefLibrary {
    let tech = std::fs::read(corpus_path("def/tech.lef")).unwrap();
    let cells = std::fs::read(corpus_path("def/cells.lef")).unwrap();
    let strip = |b: &[u8]| -> Vec<u8> {
        let s = String::from_utf8_lossy(b);
        s.replace("END LIBRARY", "").into_bytes()
    };
    let mut combined = strip(&tech);
    combined.extend_from_slice(b"\n");
    combined.extend_from_slice(&strip(&cells));
    combined.extend_from_slice(b"\nEND LIBRARY\n");
    read_lef_full(&combined).expect("parse combined lef")
}

fn strip_oracle_meta(v: Value) -> Value {
    match v {
        Value::Object(mut m) => {
            m.remove("klayout_db_version");
            Value::Object(m)
        }
        other => other,
    }
}

fn dump_rust_fixture(name: &str) -> Value {
    let lef = build_lib();
    let def_bytes = std::fs::read(corpus_path(&format!("def/{name}.def"))).unwrap();
    let design = read_def_full(&def_bytes, &lef.library, Some(&lef)).expect("parse def");
    let top = design.top.expect("DEF DESIGN top cell");
    lefdef_klayout_parity_dump(&lef.library, top)
}

#[test]
fn lefdef_layout_matches_klayout_db() {
    for name in FIXTURES {
        let json_path = corpus_path(&format!("klayout_lefdef/{name}.json"));
        if !json_path.exists() {
            eprintln!(
                "skipping klayout_lefdef::{name}: run\n\
                 python validation/oracle_klayout_lefdef.py"
            );
            return;
        }
        let exp_raw: Value =
            serde_json::from_slice(&std::fs::read(&json_path).unwrap()).unwrap();
        let expected = strip_oracle_meta(exp_raw);
        let got = dump_rust_fixture(name);
        if got != expected {
            panic!(
                "klayout_lefdef::{name} mismatch\n--- klayout-rs ---\n{}\n\n--- KLayout db ---\n{}",
                serde_json::to_string_pretty(&got).unwrap(),
                serde_json::to_string_pretty(&expected).unwrap(),
            );
        }
    }
}
