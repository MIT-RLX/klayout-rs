//! stdin: JSON `{ "source": [x,y], "sinks_ck_dbu": [["FF1",x,y], ...], "dme_config": { "allow_detour": bool } }`
//! stdout: one JSON object (`buffer_count`, `skew`, …) matching `cts_dme` corpus metrics.

use klayout_core::Point;
use klayout_cts::{synthesise_clock_tree_dme, ClockSink, DmeConfig};
use serde::Deserialize;
use serde_json::{json, Value};
use smol_str::SmolStr;
use std::io::Read;

#[derive(Debug, Deserialize)]
struct InFixture {
    source: [i64; 2],
    sinks_ck_dbu: Vec<(String, i64, i64)>,
    #[serde(default)]
    dme_config: Option<DmeCfgIn>,
}

#[derive(Debug, Deserialize)]
struct DmeCfgIn {
    #[serde(default = "truth")]
    allow_detour: bool,
}

fn truth() -> bool {
    true
}

fn metrics_value(tree: &klayout_cts::ClockTree) -> Value {
    let mut paths: Vec<Value> = tree
        .sink_path_lengths
        .iter()
        .map(|(n, l)| json!([n.as_str(), l]))
        .collect();
    paths.sort_by(|a, b| {
        let aa = &a.as_array().unwrap()[0].as_str().unwrap();
        let ab = &b.as_array().unwrap()[0].as_str().unwrap();
        aa.cmp(ab)
    });

    let mut internal: Vec<Value> = tree
        .branches
        .iter()
        .filter(|b| b.parent.is_some())
        .map(|b| json!([b.at.x, b.at.y, b.detour_to_parent]))
        .collect();
    internal.sort_by_key(|v| {
        let a = v.as_array().unwrap();
        (
            a[0].as_i64().unwrap(),
            a[1].as_i64().unwrap(),
            a[2].as_i64().unwrap(),
        )
    });

    json!({
        "buffer_count": tree.buffer_count,
        "skew": tree.skew(),
        "total_wire_length": tree.total_wire_length(),
        "sink_path_lengths": paths,
        "internal_buffers": internal,
    })
}

fn main() {
    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .expect("read stdin");
    let spec: InFixture = serde_json::from_str(buf.trim()).expect("stdin JSON parse");
    let cfg = spec
        .dme_config
        .map(|d| DmeConfig {
            allow_detour: d.allow_detour,
        })
        .unwrap_or_default();
    let sinks: Vec<ClockSink> = spec
        .sinks_ck_dbu
        .into_iter()
        .map(|(n, x, y)| ClockSink {
            name: SmolStr::from(n.as_str()),
            at: Point::new(x, y),
        })
        .collect();

    let tree = synthesise_clock_tree_dme(
        Point::new(spec.source[0], spec.source[1]),
        &sinks,
        &cfg,
    );

    println!("{}", metrics_value(&tree));
}
