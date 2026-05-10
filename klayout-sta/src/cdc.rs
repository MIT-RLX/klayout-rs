//! Clock-domain crossing (CDC) detection.
//!
//! In multi-clock designs, data paths between flops on different
//! clocks are inherently asynchronous and need synchronisers (e.g.
//! 2-FF synchroniser) to avoid metastability. Forgetting one is a
//! silicon-bug-level miss. CDC analysis flags every register-to-
//! register path that crosses domains without a synchroniser.
//!
//! v1: each [`Node`] gets an optional `clock_domain` attribute; we
//! walk the graph and report any net path connecting two endpoints
//! with different non-`None` clock domains.

use crate::graph::{NodeId, TimingGraph};
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct CdcViolation {
    pub from: NodeId,
    pub to: NodeId,
    pub from_clock: SmolStr,
    pub to_clock: SmolStr,
}

/// Find every direct edge whose endpoints sit on different clock
/// domains. `clock_domain[node_id]` is the per-node clock id (use
/// empty string for "no clock / combinational").
pub fn find_cdc_violations(
    graph: &TimingGraph,
    clock_domain: &[SmolStr],
) -> Vec<CdcViolation> {
    let mut out = Vec::new();
    for e in &graph.edges {
        let from_idx = e.from.0 as usize;
        let to_idx = e.to.0 as usize;
        let fc = clock_domain.get(from_idx);
        let tc = clock_domain.get(to_idx);
        if let (Some(f), Some(t)) = (fc, tc) {
            if !f.is_empty() && !t.is_empty() && f != t {
                out.push(CdcViolation {
                    from: e.from,
                    to: e.to,
                    from_clock: f.clone(),
                    to_clock: t.clone(),
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeKind, NodeKind, TimingGraph};

    #[test]
    fn same_clock_domain_no_violation() {
        let mut g = TimingGraph::new();
        let a = g.add_node("a", NodeKind::CellInput);
        let b = g.add_node("b", NodeKind::CellOutput);
        g.add_edge(a, b, EdgeKind::CellArc, 0.1);
        g.finalize();
        let domains = vec![SmolStr::from("clk1"), SmolStr::from("clk1")];
        assert!(find_cdc_violations(&g, &domains).is_empty());
    }

    #[test]
    fn cross_domain_edge_flagged() {
        let mut g = TimingGraph::new();
        let a = g.add_node("a", NodeKind::CellInput);
        let b = g.add_node("b", NodeKind::CellOutput);
        g.add_edge(a, b, EdgeKind::CellArc, 0.1);
        g.finalize();
        let domains = vec![SmolStr::from("clk1"), SmolStr::from("clk2")];
        let viols = find_cdc_violations(&g, &domains);
        assert_eq!(viols.len(), 1);
        assert_eq!(viols[0].from_clock.as_str(), "clk1");
        assert_eq!(viols[0].to_clock.as_str(), "clk2");
    }

    #[test]
    fn empty_domain_treated_as_combinational() {
        let mut g = TimingGraph::new();
        let a = g.add_node("a", NodeKind::CellInput);
        let b = g.add_node("b", NodeKind::CellOutput);
        g.add_edge(a, b, EdgeKind::CellArc, 0.1);
        g.finalize();
        let domains = vec![SmolStr::from(""), SmolStr::from("clk1")];
        assert!(find_cdc_violations(&g, &domains).is_empty());
    }
}
