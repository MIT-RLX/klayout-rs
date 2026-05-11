//! DME clock-tree synthesis regression suite.
//!
//! `validation/corpus/cts_dme.json` stores golden topology metrics (skew,
//! wire length, sorted sink delays, sorted internal-buffer coordinates)
//! for [`klayout_cts::synthesise_clock_tree_dme`]. Regenerate after
//! intentional DME changes:
//!
//! ```text
//! cargo run -p klayout-cts --example dump_cts_dme_corpus > validation/corpus/cts_dme.json
//! ```
//!
//! OpenROAD `triton_cts` buffering is exercised in `--test cts_downstream`
//! (`corpus/cts_downstream/`) separately from abstract DME.

use klayout_core::Point;
use klayout_cts::{synthesise_clock_tree_dme, ClockSink, ClockTree, DmeConfig};
use klayout_validate::corpus_path;
use serde::Deserialize;
use serde_json::{json, Value};
use smol_str::SmolStr;

#[derive(Debug, Deserialize)]
struct CtsSuite {
    cases: Vec<CtsCase>,
    dme_config: Option<DumpedDmeConfig>,
}

#[derive(Debug, Deserialize)]
struct DumpedDmeConfig {
    #[serde(default = "truth")]
    allow_detour: bool,
}

fn truth() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct CtsCase {
    name: String,
    source: [i64; 2],
    sinks: Vec<(String, i64, i64)>,
    expect: Value,
}

fn load_suite() -> CtsSuite {
    let path = corpus_path("cts_dme.json");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("invalid cts_dme corpus: {e}"))
}

fn metrics_json(tree: &ClockTree) -> Value {
    let mut paths: Vec<Value> = tree
        .sink_path_lengths
        .iter()
        .map(|(n, l)| json!([n.as_str(), l]))
        .collect();
    paths.sort_by(|a, b| {
        let aa = &a.as_array().expect("sink path")[0]
            .as_str()
            .expect("sink name string");
        let ab = &b.as_array().expect("sink path")[0]
            .as_str()
            .expect("sink name string");
        aa.cmp(ab)
    });

    let mut internal: Vec<Value> = tree
        .branches
        .iter()
        .filter(|b| b.parent.is_some())
        .map(|b| json!([b.at.x, b.at.y, b.detour_to_parent]))
        .collect();
    internal.sort_by_key(|v| {
        let a = v.as_array().expect("buffer triple");
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

#[test]
fn dme_matches_golden_corpus() {
    let suite = load_suite();
    let cfg = suite
        .dme_config
        .map(|d| DmeConfig {
            allow_detour: d.allow_detour,
        })
        .unwrap_or_default();

    for case in &suite.cases {
        let sinks: Vec<ClockSink> = case
            .sinks
            .iter()
            .map(|(name, x, y)| ClockSink {
                name: SmolStr::from(name.as_str()),
                at: Point::new(*x, *y),
            })
            .collect();

        let tree = synthesise_clock_tree_dme(
            Point::new(case.source[0], case.source[1]),
            &sinks,
            &cfg,
        );
        let got = metrics_json(&tree);
        assert_eq!(
            got, case.expect,
            "case {} DME metrics drift — refresh corpus with `cargo run -p klayout-cts --example dump_cts_dme_corpus` if intentional",
            case.name
        );
    }
    eprintln!("{} DME CTS golden cases ok", suite.cases.len());
}
