//! DEF parser parity vs OpenDB. Compares design name + die area +
//! instance list (name, master, x, y in DBU) + net connections.
//!
//! Regenerate corpus with `python validation/oracle_external.py def`.

use klayout_lef::{
    read_def_full, read_lef_full,
    types::{PinDirection, PinUse, RouteSegment},
};
use klayout_validate::corpus_path;
use serde_json::{json, Value};

const FIXTURES: &[&str] = &[
    "inv_chain",
    "mixed_cells",
    "orientations",
    "nand_chain",
    "seq_design",
    "with_routes",
];

fn build_lib() -> klayout_core::Library {
    // The LEF reader builds a fresh Library per call. Concatenate the
    // two LEFs into a single bytestream after stripping each `END
    // LIBRARY` so the parser sees one continuous library — the same
    // shape OpenDB ends up with after two separate `read_lef` calls.
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
    read_lef_full(&combined).expect("parse combined lef").library
}

/// Strip empty arrays and recurse into nested objects/arrays. The
/// OpenDB oracle is inconsistent across fixtures about emitting
/// `"bpins": []` vs omitting the key entirely (likewise for empty
/// `"wires"` per net). Both forms are semantically identical, so we
/// canonicalize both sides to the omitting form before comparing.
fn canonicalize(v: Value) -> Value {
    match v {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, val) in map {
                let val = canonicalize(val);
                let drop = matches!(&val, Value::Array(a) if a.is_empty());
                if !drop {
                    out.insert(k, val);
                }
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(canonicalize).collect()),
        other => other,
    }
}

fn dump_to_json(name: &str) -> Value {
    let lib = build_lib();
    let def_bytes = std::fs::read(corpus_path(&format!("def/{name}.def"))).unwrap();
    let design = read_def_full(&def_bytes, &lib).expect("parse def");
    let top_id = design.top.expect("def has no top cell");
    let cell = lib.get(top_id);

    let die = design.diearea.map(|b| {
        json!([b.min.x, b.min.y, b.max.x, b.max.y])
    }).unwrap_or(json!([0, 0, 0, 0]));

    let mut instances: Vec<Value> = cell
        .instances()
        .iter()
        .map(|inst| {
            let name = inst
                .properties
                .get("def_name")
                .and_then(|v| match v {
                    klayout_core::PropertyValue::String(s) => Some(s.to_string()),
                    _ => None,
                })
                .unwrap_or_default();
            let master = lib.get(inst.cell).name().as_str().to_string();
            json!({
                "name": name,
                "master": master,
                "x_dbu": inst.trans.disp.x,
                "y_dbu": inst.trans.disp.y,
            })
        })
        .collect();
    instances.sort_by(|a, b| a["name"].as_str().unwrap().cmp(b["name"].as_str().unwrap()));

    let mut nets: Vec<Value> = design
        .nets
        .iter()
        .map(|n| {
            let mut conns: Vec<String> = n
                .connects
                .iter()
                .filter_map(|c| {
                    let inst = c.instance.as_ref()?;
                    Some(format!("{}:{}", inst, c.pin))
                })
                .collect();
            conns.sort();
            // Wires: each two-vertex segment in `Wire` polylines.
            let mut wires: Vec<Value> = Vec::new();
            for seg in &n.segments {
                if let RouteSegment::Wire { layer, points, .. } = seg {
                    for w in points.windows(2) {
                        wires.push(json!({
                            "layer": layer.as_str(),
                            "x1": w[0].x, "y1": w[0].y,
                            "x2": w[1].x, "y2": w[1].y,
                        }));
                    }
                }
            }
            wires.sort_by_key(|w| {
                (
                    w["layer"].as_str().unwrap().to_string(),
                    w["x1"].as_i64().unwrap(),
                    w["y1"].as_i64().unwrap(),
                    w["x2"].as_i64().unwrap(),
                    w["y2"].as_i64().unwrap(),
                )
            });
            // Omit `wires` when empty so the JSON shape matches the
            // OpenDB oracle, which doesn't emit the field for nets
            // without routing geometry.
            if wires.is_empty() {
                json!({ "name": n.name.as_str(), "connections": conns })
            } else {
                json!({ "name": n.name.as_str(), "connections": conns, "wires": wires })
            }
        })
        .collect();
    nets.sort_by(|a, b| a["name"].as_str().unwrap().cmp(b["name"].as_str().unwrap()));

    let mut bpins: Vec<Value> = design
        .pins
        .iter()
        .map(|p| {
            let dir = p.direction.map(|d| match d {
                PinDirection::Input => "INPUT",
                PinDirection::Output => "OUTPUT",
                PinDirection::Inout => "INOUT",
                PinDirection::Feedthru => "FEEDTHRU",
            });
            let usage = p.use_.map(|u| match u {
                PinUse::Signal => "SIGNAL",
                PinUse::Power => "POWER",
                PinUse::Ground => "GROUND",
                PinUse::Clock => "CLOCK",
                PinUse::Analog => "ANALOG",
                PinUse::Reset => "RESET",
                PinUse::Scan => "SCAN",
                PinUse::Tieoff => "TIEOFF",
            });
            json!({ "name": p.name.as_str(), "direction": dir, "use": usage })
        })
        .collect();
    bpins.sort_by(|a, b| a["name"].as_str().unwrap().cmp(b["name"].as_str().unwrap()));

    // Omit `bpins` when empty (matches OpenDB oracle's omission of
    // the field on designs with no PINS section).
    if bpins.is_empty() {
        json!({
            "design": design.design_name.as_str(),
            "die": die,
            "instances": instances,
            "nets": nets,
        })
    } else {
        json!({
            "design": design.design_name.as_str(),
            "die": die,
            "instances": instances,
            "bpins": bpins,
            "nets": nets,
        })
    }
}

#[test]
fn def_parser_matches_opendb() {
    for name in FIXTURES {
        let def_path = corpus_path(&format!("def/{name}.def"));
        let json_path = corpus_path(&format!("def/{name}.json"));
        if !def_path.exists() || !json_path.exists() {
            eprintln!(
                "skipping def::{name}: corpus missing.\n\
                 Regenerate with: python validation/oracle_external.py def"
            );
            return;
        }
        let got = canonicalize(dump_to_json(name));
        let exp_bytes = std::fs::read(&json_path).expect("read json");
        let expected: Value =
            canonicalize(serde_json::from_slice(&exp_bytes).expect("parse json"));
        if got != expected {
            let g = serde_json::to_string_pretty(&got).unwrap();
            let e = serde_json::to_string_pretty(&expected).unwrap();
            panic!("def::{name} mismatch\n--- got ---\n{g}\n\n--- expected (OpenDB) ---\n{e}\n");
        }
    }
}
