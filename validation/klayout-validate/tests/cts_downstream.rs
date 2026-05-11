//! CTS downstream parity: OpenROAD (`read_def` + `read_liberty` + `clock_tree_synthesis`)
//! vs klayout-rs geometry (`klayout_lef` CK centroids) + abstract DME (`klayout_cts`).
//!
//! Regenerate corpus (Docker + pinned `klayout-cts` helper):
//!
//! ```text
//! python validation/oracle_external.py cts_downstream
//! ```

use klayout_core::Point;
use klayout_cts::{synthesise_clock_tree_dme, ClockSink, ClockTree, DmeConfig};
use klayout_lef::{read_def_full, read_lef_full};
use klayout_validate::{
    corpus_path,
    cts_downstream::ck_sink_centers_placement_north,
};
use serde::Deserialize;
use serde_json::{json, Value};
use smol_str::SmolStr;
use std::collections::BTreeSet;

#[derive(Debug, Deserialize)]
struct Suite {
    #[allow(dead_code)]
    suite: String,
    dme_config: Option<DumpedDmeConfig>,
    cases: Vec<Case>,
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
struct Case {
    name: String,
    def_file: String,
    clock_net: String,
    ck_pin: String,
    dme_source_dbu: [i64; 2],
    sinks_ck_dbu: Vec<(String, i64, i64)>,
    openroad_after_cts: Value,
    dme_expect: Value,
}

fn load_suite() -> Suite {
    let path = corpus_path("cts_downstream/cts_downstream.json");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_slice(&bytes).unwrap_or_else(|e| panic!("invalid cts_downstream corpus: {e}"))
}

fn combined_placement_lef_bytes(subdir: &str) -> Vec<u8> {
    let tech = std::fs::read(corpus_path(&format!("{subdir}/tech.lef"))).unwrap();
    let cells = std::fs::read(corpus_path(&format!("{subdir}/cells.lef"))).unwrap();
    let strip = |b: &[u8]| -> Vec<u8> {
        let s = String::from_utf8_lossy(b);
        s.replace("END LIBRARY", "").into_bytes()
    };
    let mut combined = strip(&tech);
    combined.extend_from_slice(b"\n");
    combined.extend_from_slice(&strip(&cells));
    combined.extend_from_slice(b"\nEND LIBRARY\n");
    combined
}

fn metrics_json(tree: &ClockTree) -> Value {
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

/// Frozen `report_cts` excerpt in the corpus: fixed key set plus internal consistency checks.
fn assert_openroad_after_cts_parity(or: &Value, case_name: &str, n_sinks: usize) {
    const KEYS: [&str; 5] = [
        "buffer_usage",
        "buffers_inserted",
        "clock_roots",
        "clock_subnets",
        "sinks",
    ];
    let obj = or
        .as_object()
        .unwrap_or_else(|| panic!("{}: openroad_after_cts must be object", case_name));
    let got: BTreeSet<_> = obj.keys().map(|k| k.as_str()).collect();
    assert_eq!(
        got,
        KEYS.iter().copied().collect::<BTreeSet<_>>(),
        "{}: openroad_after_cts keys must match corpus contract — regen with python3 validation/oracle_external.py cts_downstream",
        case_name
    );

    assert_eq!(
        obj["clock_roots"].as_i64(),
        Some(1),
        "{}: clock_roots",
        case_name
    );
    assert_eq!(
        obj["sinks"].as_i64(),
        Some(n_sinks as i64),
        "{}: sinks vs sink list length",
        case_name
    );

    let buffers_inserted = obj["buffers_inserted"].as_i64().unwrap_or_else(|| {
        panic!(
            "{}: buffers_inserted must be integer",
            case_name
        )
    });
    assert!(
        buffers_inserted > 0,
        "{}: buffers_inserted",
        case_name
    );

    let bu = obj["buffer_usage"]
        .as_object()
        .unwrap_or_else(|| panic!("{}: buffer_usage must be object", case_name));
    assert!(
        !bu.is_empty(),
        "{}: buffer_usage non-empty",
        case_name
    );

    let usage_sum: i64 = bu
        .values()
        .map(|v| {
            let n = v.as_i64().unwrap_or_else(|| {
                panic!(
                    "{}: buffer_usage counts must be integers",
                    case_name
                )
            });
            assert!(n > 0, "{}: buffer_usage count {n}", case_name);
            n
        })
        .sum();
    assert_eq!(
        usage_sum, buffers_inserted,
        "{}: sum(buffer_usage) must equal Buffers Inserted (report_cts)",
        case_name
    );

    assert!(
        obj["clock_subnets"].as_i64().is_some_and(|n| n > 0),
        "{}: clock_subnets",
        case_name
    );
}

#[test]
fn downstream_clock_sinks_dme_and_frozen_openroad_report() {
    let suite = load_suite();
    let cfg = suite
        .dme_config
        .map(|d| DmeConfig {
            allow_detour: d.allow_detour,
        })
        .unwrap_or_default();

    let lef_b = combined_placement_lef_bytes("cts_downstream");
    let lef_bundle = read_lef_full(&lef_b).expect("parse CTS downstream LEF");
    let placement_lib = &lef_bundle.library;

    for case in suite.cases {
        let design = read_def_full(
            &std::fs::read(corpus_path(&format!(
                "cts_downstream/{}",
                case.def_file
            )))
            .unwrap(),
            placement_lib,
            Some(&lef_bundle),
        )
        .unwrap_or_else(|e| panic!("parse {} DEF: {e}", case.name));

        let got_sinks = ck_sink_centers_placement_north(
            &lef_bundle,
            placement_lib,
            &design,
            &case.clock_net,
            &case.ck_pin,
        )
        .unwrap_or_else(|e| panic!("{} extract sinks: {e}", case.name));

        let expect_sinks: Vec<(String, Point)> = case
            .sinks_ck_dbu
            .iter()
            .map(|(n, x, y)| (n.clone(), Point::new(*x, *y)))
            .collect();

        assert_eq!(
            got_sinks, expect_sinks,
            "{}: CK centroid extraction vs OpenROAD bbox oracle",
            case.name
        );

        let sinks: Vec<ClockSink> = got_sinks
            .into_iter()
            .map(|(n, at)| ClockSink {
                name: SmolStr::from(n.as_str()),
                at,
            })
            .collect();
        let tree = synthesise_clock_tree_dme(
            Point::new(case.dme_source_dbu[0], case.dme_source_dbu[1]),
            &sinks,
            &cfg,
        );
        let got_dme = metrics_json(&tree);
        assert_eq!(
            got_dme, case.dme_expect,
            "{}: DME regression (centroid tap from oracle sinks)",
            case.name
        );

        assert_openroad_after_cts_parity(
            &case.openroad_after_cts,
            &case.name,
            case.sinks_ck_dbu.len(),
        );
    }
}
