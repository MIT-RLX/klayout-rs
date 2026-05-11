//! Emit `validation/corpus/cts_dme.json` for differential regression testing.
//!
//! From the repo root:
//! ```text
//! cargo run -p klayout-cts --example dump_cts_dme_corpus > validation/corpus/cts_dme.json
//! ```

use klayout_core::Point;
use klayout_cts::{synthesise_clock_tree_dme, ClockSink, DmeConfig};
use serde_json::{json, Value};
use smol_str::SmolStr;

fn sink(name: &str, x: i64, y: i64) -> ClockSink {
    ClockSink {
        name: SmolStr::from(name),
        at: Point::new(x, y),
    }
}

fn case_expect(source: Point, sinks: &[ClockSink], cfg: &DmeConfig) -> Value {
    let t = synthesise_clock_tree_dme(source, sinks, cfg);
    let mut paths: Vec<Value> = t
        .sink_path_lengths
        .iter()
        .map(|(n, l)| json!([n.as_str(), l]))
        .collect();
    paths.sort_by(|a, b| {
        let aa = &a.as_array().unwrap()[0].as_str().unwrap();
        let ab = &b.as_array().unwrap()[0].as_str().unwrap();
        aa.cmp(ab)
    });

    let mut internal: Vec<Value> = t
        .branches
        .iter()
        .filter(|b| b.parent.is_some())
        .map(|b| json!([b.at.x, b.at.y, b.detour_to_parent]))
        .collect();
    internal.sort_by_key(|v| {
        let a = v.as_array().unwrap();
        (a[0].as_i64().unwrap(), a[1].as_i64().unwrap(), a[2].as_i64().unwrap())
    });

    json!({
        "buffer_count": t.buffer_count,
        "skew": t.skew(),
        "total_wire_length": t.total_wire_length(),
        "sink_path_lengths": paths,
        "internal_buffers": internal,
    })
}

fn main() {
    let cfg = DmeConfig::default();
    let cases = vec![
        json!({
            "name": "empty",
            "source": [0, 0],
            "sinks": Value::Array(vec![]),
            "expect": case_expect(Point::new(0, 0), &[], &cfg),
        }),
        json!({
            "name": "single_ff",
            "source": [0, 0],
            "sinks": [["ff0", 100, 0]],
            "expect": case_expect(Point::new(0, 0), &[sink("ff0", 100, 0)], &cfg),
        }),
        json!({
            "name": "symmetric_x",
            "source": [0, 0],
            "sinks": [["ff0", -100, 0], ["ff1", 100, 0]],
            "expect": case_expect(Point::new(0, 0),
                &[sink("ff0", -100, 0), sink("ff1", 100, 0)], &cfg),
        }),
        json!({
            "name": "top_down_y_pair",
            "source": [50, 50],
            "sinks": [["a", 0, 0], ["b", 0, 100]],
            "expect": case_expect(Point::new(50, 50),
                &[sink("a", 0, 0), sink("b", 0, 100)], &cfg),
        }),
        json!({
            "name": "three_collinear",
            "source": [0, 0],
            "sinks": [["a", 0, 0], ["b", 10, 0], ["c", 1000, 0]],
            "expect": case_expect(Point::new(0, 0),
                &[sink("a", 0, 0), sink("b", 10, 0), sink("c", 1000, 0)], &cfg),
        }),
        {
            let sinks: Vec<ClockSink> = (0..16)
                .map(|i| ClockSink {
                    name: SmolStr::from(format!("ff{}", i)),
                    at: Point::new((i % 4) * 100, (i / 4) * 100),
                })
                .collect();
            let sink_json: Vec<Value> = (0..16)
                .map(|i| json!([format!("ff{}", i), (i % 4) * 100, (i / 4) * 100]))
                .collect();
            json!({
                "name": "grid_4x4",
                "source": [150, 150],
                "sinks": sink_json,
                "expect": case_expect(Point::new(150, 150), &sinks, &cfg),
            })
        },
    ];

    let root = json!({
        "suite": "cts_dme",
        "description": "Golden metrics for klayout-cts DME (Chao/Hsu/Ho). Regenerate with: cargo run -p klayout-cts --example dump_cts_dme_corpus",
        "dme_config": { "allow_detour": cfg.allow_detour },
        "cases": cases,
    });
    println!("{}", serde_json::to_string_pretty(&root).unwrap());
}
