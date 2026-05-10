//! STA sanity tests over hand-built timing graphs.

use klayout_sta::graph::{EdgeKind, NodeKind, TimingGraph};
use klayout_sta::report::{trace_worst_path, SlackReport};
use klayout_sta::{compute_arrivals, compute_required, compute_slacks};

fn linear_chain() -> TimingGraph {
    // PI → C0:in → C0:out → C1:in → C1:out → PO with cell delays.
    let mut g = TimingGraph::new();
    let pi = g.add_node("(input)/A", NodeKind::PrimaryInput);
    let c0_in = g.add_node("u0/A", NodeKind::CellInput);
    let c0_out = g.add_node("u0/Y", NodeKind::CellOutput);
    let c1_in = g.add_node("u1/A", NodeKind::CellInput);
    let c1_out = g.add_node("u1/Y", NodeKind::CellOutput);
    let po = g.add_node("(output)/Y", NodeKind::PrimaryOutput);

    g.add_edge(pi, c0_in, EdgeKind::Net, 0.05);
    g.add_edge(c0_in, c0_out, EdgeKind::CellArc, 0.20);
    g.add_edge(c0_out, c1_in, EdgeKind::Net, 0.10);
    g.add_edge(c1_in, c1_out, EdgeKind::CellArc, 0.20);
    g.add_edge(c1_out, po, EdgeKind::Net, 0.05);
    g.finalize();
    g
}

#[test]
fn arrivals_accumulate_along_chain() {
    let g = linear_chain();
    let arr = compute_arrivals(&g, &[]).unwrap();
    // Last node arrival: 0.05 + 0.20 + 0.10 + 0.20 + 0.05 = 0.60.
    assert!((arr[arr.len() - 1] - 0.60).abs() < 1e-9);
}

#[test]
fn required_propagates_backward() {
    let g = linear_chain();
    let arr = compute_arrivals(&g, &[]).unwrap();
    // Clock period 1.0 → required at PO = 1.0; backward slack = 0.40.
    let req = compute_required(&g, &[], 1.0).unwrap();
    let slacks = compute_slacks(&arr, &req);
    let po_idx = arr.len() - 1;
    assert!((slacks[po_idx] - 0.40).abs() < 1e-9);
    // PI slack should also be 0.40 (uniform along chain — clean path).
    assert!((slacks[0] - 0.40).abs() < 1e-9);
}

#[test]
fn negative_slack_when_period_too_tight() {
    let g = linear_chain();
    let arr = compute_arrivals(&g, &[]).unwrap();
    let req = compute_required(&g, &[], 0.50).unwrap(); // < 0.60 path
    let slacks = compute_slacks(&arr, &req);
    let worst = slacks.iter().cloned().fold(f64::INFINITY, f64::min);
    assert!(worst < 0.0, "expected negative slack, got {worst}");
}

#[test]
fn parallel_paths_take_max_arrival() {
    let mut g = TimingGraph::new();
    let pi = g.add_node("(input)/A", NodeKind::PrimaryInput);
    let mid = g.add_node("(input)/M", NodeKind::PrimaryInput);
    let merged = g.add_node("u/Y", NodeKind::CellOutput);
    let po = g.add_node("(output)/Y", NodeKind::PrimaryOutput);
    g.add_edge(pi, merged, EdgeKind::CellArc, 0.30);
    g.add_edge(mid, merged, EdgeKind::CellArc, 0.50); // bigger
    g.add_edge(merged, po, EdgeKind::Net, 0.10);
    g.finalize();

    let arr = compute_arrivals(&g, &[]).unwrap();
    // Arrival at po: max(0+0.3, 0+0.5) + 0.1 = 0.6.
    assert!((arr[po.0 as usize] - 0.60).abs() < 1e-9);
}

#[test]
fn slack_report_sorts_worst_first() {
    let g = linear_chain();
    let arr = compute_arrivals(&g, &[]).unwrap();
    let req = compute_required(&g, &[], 0.30).unwrap();
    let report = SlackReport::build(&g, &arr, &req);
    assert!(report.worst_slack().unwrap() < 0.0);
    let top3 = report.worst_n(3);
    assert!(top3.len() >= 3);
    // First entry has the most negative slack.
    for e in top3.windows(2) {
        assert!(e[0].slack <= e[1].slack);
    }
}

#[test]
fn worst_path_trace_recovers_chain() {
    let g = linear_chain();
    let arr = compute_arrivals(&g, &[]).unwrap();
    // Find the PO node and trace.
    let po = g
        .primary_outputs()
        .next()
        .expect("primary output exists");
    let path = trace_worst_path(&g, &arr, po);
    // 6 nodes: PI, c0_in, c0_out, c1_in, c1_out, PO.
    assert_eq!(path.len(), 6);
}
