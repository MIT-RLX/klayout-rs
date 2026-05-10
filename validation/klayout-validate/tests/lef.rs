//! LEF parser parity vs OpenDB (`read_lef` inside the
//! `klayout-rs-oracle:latest` Docker image).
//!
//! Compares macro name + size (in DBU) + pin direction/use. Regenerate
//! with `python validation/oracle_external.py lef`.

use klayout_lef::{
    read_lef_full,
    types::{LayerType, PinDirection, PinUse, PortShape},
};
use klayout_validate::corpus_path;
use serde_json::{json, Value};

fn dump_to_json() -> Value {
    let tech = std::fs::read(corpus_path("lef/tech.lef")).unwrap();
    let cells = std::fs::read(corpus_path("lef/cells.lef")).unwrap();
    let lib_t = read_lef_full(&tech).expect("parse tech lef");
    let lib_c = read_lef_full(&cells).expect("parse cells lef");

    let mut layers: Vec<Value> = lib_t
        .layers
        .iter()
        .chain(lib_c.layers.iter())
        .map(|l| {
            let ty = l.layer_type.map(|t| match t {
                LayerType::Routing => "ROUTING",
                LayerType::Cut => "CUT",
                LayerType::Masterslice => "MASTERSLICE",
                LayerType::Overlap => "OVERLAP",
                LayerType::Implant => "IMPLANT",
                LayerType::Other => "NONE",
            });
            json!({ "name": l.name.as_str(), "type": ty })
        })
        .collect();
    layers.sort_by(|a, b| a["name"].as_str().unwrap().cmp(b["name"].as_str().unwrap()));

    let all_macros: Vec<_> = lib_t.macros.iter().chain(lib_c.macros.iter()).collect();
    let mut macros: Vec<Value> = all_macros
        .iter()
        .map(|m| {
            let (w_um, h_um) = m.size.unwrap_or((0.0, 0.0));
            let mut pins: Vec<Value> = m
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
                    let mut ports: Vec<Value> = p
                        .geometry
                        .shapes
                        .iter()
                        .filter_map(|(layer, shape)| match shape {
                            PortShape::Rect(b) => Some(json!({
                                "layer": layer.as_str(),
                                "xmin": b.min.x,
                                "ymin": b.min.y,
                                "xmax": b.max.x,
                                "ymax": b.max.y,
                            })),
                            // Polygons not yet asserted; corpus is rect-only.
                            PortShape::Polygon(_) => None,
                        })
                        .collect();
                    ports.sort_by(|a, b| {
                        let ka = (
                            a["layer"].as_str().unwrap().to_string(),
                            a["xmin"].as_i64().unwrap(),
                            a["ymin"].as_i64().unwrap(),
                        );
                        let kb = (
                            b["layer"].as_str().unwrap().to_string(),
                            b["xmin"].as_i64().unwrap(),
                            b["ymin"].as_i64().unwrap(),
                        );
                        ka.cmp(&kb)
                    });
                    json!({
                        "name": p.name.as_str(),
                        "direction": dir,
                        "use": usage,
                        "ports": ports,
                    })
                })
                .collect();
            pins.sort_by(|a, b| a["name"].as_str().unwrap().cmp(b["name"].as_str().unwrap()));
            json!({
                "name": m.name.as_str(),
                "width_dbu": (w_um * 1000.0).round() as i64,
                "height_dbu": (h_um * 1000.0).round() as i64,
                "pins": pins,
            })
        })
        .collect();
    macros.sort_by(|a, b| a["name"].as_str().unwrap().cmp(b["name"].as_str().unwrap()));
    json!({ "layers": layers, "macros": macros })
}

#[test]
fn lef_parser_matches_opendb() {
    let json_path = corpus_path("lef/lef.json");
    if !json_path.exists() {
        eprintln!(
            "skipping lef test: corpus missing.\n\
             Regenerate with: python validation/oracle_external.py lef"
        );
        return;
    }
    let got = dump_to_json();
    let exp_bytes = std::fs::read(&json_path).expect("read json");
    let expected: Value = serde_json::from_slice(&exp_bytes).expect("parse json");
    if got != expected {
        let g = serde_json::to_string_pretty(&got).unwrap();
        let e = serde_json::to_string_pretty(&expected).unwrap();
        panic!("lef mismatch\n--- got ---\n{g}\n\n--- expected (OpenDB) ---\n{e}\n");
    }
}
