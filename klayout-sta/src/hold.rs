//! Hold-time checks.
//!
//! Setup checks ensure data arrives *before* the next clock edge
//! (`arrival ≤ required = clock_period - setup_skew`). Hold checks
//! ensure data arrives *after* the previous clock edge —
//! specifically, `arrival ≥ hold_skew`. Negative slack on a hold
//! endpoint means data raced through faster than the clock-to-data
//! hold window allowed, latching a glitched value.
//!
//! The hold check uses the same arrival graph but with a different
//! polarity: `hold_slack = arrival - required_hold` where
//! `required_hold` is the per-endpoint hold time. Production STA
//! tools use min-delay corner (fast-fast / FF) for hold; we accept
//! per-endpoint times as input so the caller selects the corner.

use crate::graph::{NodeId, TimingGraph};

/// Compute hold slack at every node. `hold_required` per node gives
/// the minimum acceptable arrival time (typically positive — data
/// must arrive *after* this point in the clock cycle).
pub fn compute_hold_slacks(arrivals: &[f64], hold_required: &[f64]) -> Vec<f64> {
    arrivals
        .iter()
        .zip(hold_required.iter())
        .map(|(arr, req)| arr - req)
        .collect()
}

/// Endpoints whose hold slack is negative (i.e. arrived too early).
pub fn hold_violations(
    graph: &TimingGraph,
    hold_slacks: &[f64],
) -> Vec<(NodeId, f64)> {
    graph
        .nodes
        .iter()
        .filter_map(|n| {
            let s = hold_slacks[n.id.0 as usize];
            if s < 0.0 {
                Some((n.id, s))
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arrival::compute_arrivals;
    use crate::graph::{EdgeKind, NodeKind, TimingGraph};

    #[test]
    fn hold_slack_zero_when_arrival_meets_requirement() {
        let mut g = TimingGraph::new();
        let pi = g.add_node("(input)/A", NodeKind::PrimaryInput);
        let po = g.add_node("(output)/Y", NodeKind::PrimaryOutput);
        g.add_edge(pi, po, EdgeKind::Net, 0.5);
        g.finalize();
        let arr = compute_arrivals(&g, &[]).unwrap();
        let hold_req = vec![0.0; g.nodes.len()];
        let slacks = compute_hold_slacks(&arr, &hold_req);
        assert!(slacks[po.0 as usize] >= 0.0);
        assert!(hold_violations(&g, &slacks).is_empty());
    }

    #[test]
    fn fast_path_violates_hold_when_required_high() {
        let mut g = TimingGraph::new();
        let pi = g.add_node("(input)/A", NodeKind::PrimaryInput);
        let po = g.add_node("(output)/Y", NodeKind::PrimaryOutput);
        g.add_edge(pi, po, EdgeKind::Net, 0.1); // fast
        g.finalize();
        let arr = compute_arrivals(&g, &[]).unwrap();
        let mut hold_req = vec![0.0; g.nodes.len()];
        hold_req[po.0 as usize] = 0.5; // require arrival ≥ 0.5
        let slacks = compute_hold_slacks(&arr, &hold_req);
        let viols = hold_violations(&g, &slacks);
        assert!(!viols.is_empty());
    }
}
