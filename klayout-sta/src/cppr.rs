//! CPPR — Common-Path Pessimism Removal.
//!
//! When a setup or hold check is evaluated between a launch flop and
//! a capture flop, the timing path threads the clock network from
//! the clock root to the launch flop's clock pin (the launch clock
//! path) and, separately, from the same root to the capture flop's
//! clock pin (the capture clock path). These two paths share a
//! prefix — the *common clock path* — which physically *is the same
//! wires and buffers*.
//!
//! In modern STA, on-chip variation (OCV) factors are applied
//! separately to launch and capture, intentionally biasing each
//! direction toward its worst case (slower for launch, faster for
//! capture in a setup check, vice versa for hold). For the
//! **common** segment, that's pessimism: the same wire cannot
//! simultaneously be slow on the launch side and fast on the
//! capture side — it has *one* delay. CPPR removes this pessimism
//! by computing the magnitude of the OCV-induced bias on the common
//! segment and adding it back to slack.
//!
//! ## What this module computes
//!
//! Given a [`TimingGraph`] with a clock-tree subgraph:
//!
//! * [`clock_arrivals`] — clock-network forward propagation from a
//!   designated clock root.
//! * [`lowest_common_clock_ancestor`] — the deepest node that lies on
//!   both the launch flop's and the capture flop's clock paths.
//!   This is the LCA used by CPPR: any node above it is on the
//!   common segment; any node below it is exclusive to one side.
//! * [`common_path_delay`] — the clock-arrival time at the LCA, i.e.
//!   the delay through the common segment.
//! * [`cppr_credit`] — given launch/capture clock pins, an early/late
//!   OCV pair `(d_early, d_late)`, returns the slack credit
//!   `(d_late − d_early) · common_path_delay`. With a single-corner
//!   model (no OCV) the credit is `0` — the function is here so a
//!   later OCV / multi-corner pass slots in without re-architecting.
//!
//! ## What this module does NOT compute
//!
//! * **Per-arc slew/load-aware OCV factors** (POCV / Liberty-OCM).
//!   The credit formula assumes a single multiplicative derate per
//!   side; the [`crate::derate`] module already supports per-depth
//!   AOCV and POCV tables that can be composed on top.
//! * **Setup/hold check evaluation.** [`crate::arrival`] handles
//!   forward / backward propagation; the CPPR credit is added by the
//!   reporting layer.
//!
//! ## Algorithm — LCA via path-set intersection
//!
//! For each clock pin we walk the clock graph back to the root and
//! record the ordered set of clock nodes traversed. The LCA is the
//! deepest node common to both ordered sets. With a tree-shaped
//! clock network this is `O(d)` per query where `d` is clock depth.
//! For meshes / clock crossbars the same code runs (we walk every
//! ancestor) but the LCA notion becomes the deepest *shared*
//! ancestor on the longest common prefix.

use crate::graph::{NodeId, TimingGraph};
use crate::Result;
use std::collections::HashSet;

/// Forward-propagate clock arrivals from `clock_root` along the
/// clock subgraph. A node belongs to the clock subgraph if it is the
/// root or is reachable from the root through edges. We intentionally
/// don't filter on edge kind — clock buffers contribute `CellArc`
/// edges; clock interconnect contributes `Net` edges — both are walked.
///
/// Returns a vector of length `g.nodes.len()`. Non-clock-network
/// nodes are left at `f64::NEG_INFINITY`.
pub fn clock_arrivals(g: &TimingGraph, clock_root: NodeId) -> Result<Vec<f64>> {
    let n = g.nodes.len();
    let mut arr = vec![f64::NEG_INFINITY; n];
    arr[clock_root.0 as usize] = 0.0;

    let order = crate::arrival::topo_sort(g)?;
    for nid in order {
        if arr[nid.0 as usize] == f64::NEG_INFINITY {
            continue;
        }
        let from = arr[nid.0 as usize];
        for e in g.outgoing(nid) {
            let cand = from + e.delay;
            let dst = e.to.0 as usize;
            if cand > arr[dst] {
                arr[dst] = cand;
            }
        }
    }
    Ok(arr)
}

/// Walk all paths from `clock_root` to `target` and collect every
/// node visited. With a DAG this is the set of ancestors of `target`
/// in the clock graph; with a tree it's exactly the unique path.
fn ancestors(g: &TimingGraph, clock_root: NodeId, target: NodeId) -> HashSet<NodeId> {
    let mut visited: HashSet<NodeId> = HashSet::new();
    let mut stack = vec![target];
    while let Some(node) = stack.pop() {
        if !visited.insert(node) {
            continue;
        }
        if node == clock_root {
            continue;
        }
        for e in g.incoming(node) {
            stack.push(e.from);
        }
    }
    visited
}

/// LCA of two clock pins under the clock root, defined as the
/// ancestor with the **largest clock arrival time** that's common to
/// both targets' ancestor sets. With a tree-shaped clock network
/// this matches the textbook tree LCA; with a DAG it's the deepest
/// shared ancestor under the timing arrival measure.
pub fn lowest_common_clock_ancestor(
    g: &TimingGraph,
    clock_root: NodeId,
    clock_arr: &[f64],
    a: NodeId,
    b: NodeId,
) -> Option<NodeId> {
    let anc_a = ancestors(g, clock_root, a);
    let anc_b = ancestors(g, clock_root, b);
    let mut best: Option<(NodeId, f64)> = None;
    for n in anc_a.intersection(&anc_b) {
        let t = clock_arr[n.0 as usize];
        if !t.is_finite() {
            continue;
        }
        match best {
            None => best = Some((*n, t)),
            Some((_, bt)) if t > bt => best = Some((*n, t)),
            _ => {}
        }
    }
    best.map(|(n, _)| n)
}

/// Clock arrival at the LCA — i.e. delay through the common clock
/// segment shared by `launch` and `capture`. Returns `0.0` if the
/// two share no ancestor (which happens for launch/capture pairs on
/// independent clock domains; CDC analysis lives in
/// [`crate::cdc`]).
pub fn common_path_delay(
    g: &TimingGraph,
    clock_root: NodeId,
    clock_arr: &[f64],
    launch: NodeId,
    capture: NodeId,
) -> f64 {
    lowest_common_clock_ancestor(g, clock_root, clock_arr, launch, capture)
        .map(|lca| clock_arr[lca.0 as usize].max(0.0))
        .unwrap_or(0.0)
}

/// Slack credit produced by removing common-path pessimism, given
/// the OCV `(d_early, d_late)` pair applied on the launch and
/// capture sides respectively. With a single corner (no OCV),
/// `d_early == d_late == 1.0` and the credit is `0` — the API still
/// returns a value so the reporting code path is neutral.
pub fn cppr_credit(
    g: &TimingGraph,
    clock_root: NodeId,
    clock_arr: &[f64],
    launch: NodeId,
    capture: NodeId,
    d_late: f64,
    d_early: f64,
) -> f64 {
    let cpd = common_path_delay(g, clock_root, clock_arr, launch, capture);
    cpd * (d_late - d_early)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeKind, NodeKind};

    /// Build a tiny clock tree:
    ///
    /// ```text
    ///       root
    ///      /    \
    ///   buf_a   buf_b
    ///    /        \
    ///  ff_a.clk   ff_b.clk
    /// ```
    fn clock_tree() -> (TimingGraph, NodeId, NodeId, NodeId) {
        let mut g = TimingGraph::new();
        let root = g.add_node("clk", NodeKind::PrimaryInput);
        let buf_a = g.add_node("buf_a/Y", NodeKind::CellOutput);
        let buf_b = g.add_node("buf_b/Y", NodeKind::CellOutput);
        let ff_a_clk = g.add_node("ff_a/CLK", NodeKind::Clock);
        let ff_b_clk = g.add_node("ff_b/CLK", NodeKind::Clock);
        g.add_edge(root, buf_a, EdgeKind::Net, 0.10);
        g.add_edge(root, buf_b, EdgeKind::Net, 0.20);
        g.add_edge(buf_a, ff_a_clk, EdgeKind::Net, 0.05);
        g.add_edge(buf_b, ff_b_clk, EdgeKind::Net, 0.05);
        g.finalize();
        (g, root, ff_a_clk, ff_b_clk)
    }

    #[test]
    fn clock_arrivals_propagate_along_clock_tree() {
        let (g, root, ff_a, ff_b) = clock_tree();
        let arr = clock_arrivals(&g, root).unwrap();
        assert!((arr[ff_a.0 as usize] - 0.15).abs() < 1e-9);
        assert!((arr[ff_b.0 as usize] - 0.25).abs() < 1e-9);
    }

    #[test]
    fn lca_is_root_for_independent_branches() {
        let (g, root, ff_a, ff_b) = clock_tree();
        let arr = clock_arrivals(&g, root).unwrap();
        let lca = lowest_common_clock_ancestor(&g, root, &arr, ff_a, ff_b);
        assert_eq!(lca, Some(root));
    }

    #[test]
    fn common_path_delay_is_root_arrival() {
        let (g, root, ff_a, ff_b) = clock_tree();
        let arr = clock_arrivals(&g, root).unwrap();
        let cpd = common_path_delay(&g, root, &arr, ff_a, ff_b);
        // The two branches diverge immediately at root, so the
        // common path delay is `clock_arr[root] = 0`.
        assert!(cpd.abs() < 1e-9);
    }

    #[test]
    fn shared_buffer_increases_common_path_delay() {
        // Now both flops share a common buffer.
        //   root → buf_shared → buf_a → ff_a/CLK
        //                     → buf_b → ff_b/CLK
        let mut g = TimingGraph::new();
        let root = g.add_node("clk", NodeKind::PrimaryInput);
        let shared = g.add_node("buf_shared/Y", NodeKind::CellOutput);
        let buf_a = g.add_node("buf_a/Y", NodeKind::CellOutput);
        let buf_b = g.add_node("buf_b/Y", NodeKind::CellOutput);
        let ff_a = g.add_node("ff_a/CLK", NodeKind::Clock);
        let ff_b = g.add_node("ff_b/CLK", NodeKind::Clock);
        g.add_edge(root, shared, EdgeKind::Net, 0.30);
        g.add_edge(shared, buf_a, EdgeKind::Net, 0.05);
        g.add_edge(shared, buf_b, EdgeKind::Net, 0.05);
        g.add_edge(buf_a, ff_a, EdgeKind::Net, 0.02);
        g.add_edge(buf_b, ff_b, EdgeKind::Net, 0.02);
        g.finalize();

        let arr = clock_arrivals(&g, root).unwrap();
        let lca = lowest_common_clock_ancestor(&g, root, &arr, ff_a, ff_b);
        assert_eq!(lca, Some(shared));
        let cpd = common_path_delay(&g, root, &arr, ff_a, ff_b);
        // shared sits at root + 0.30 = 0.30.
        assert!((cpd - 0.30).abs() < 1e-9);
    }

    #[test]
    fn cppr_credit_zero_without_ocv() {
        let (g, root, ff_a, ff_b) = clock_tree();
        let arr = clock_arrivals(&g, root).unwrap();
        let credit = cppr_credit(&g, root, &arr, ff_a, ff_b, 1.0, 1.0);
        assert_eq!(credit, 0.0);
    }

    #[test]
    fn cppr_credit_scales_with_ocv_diff_and_common_delay() {
        let mut g = TimingGraph::new();
        let root = g.add_node("clk", NodeKind::PrimaryInput);
        let shared = g.add_node("buf_shared/Y", NodeKind::CellOutput);
        let ff_a = g.add_node("ff_a/CLK", NodeKind::Clock);
        let ff_b = g.add_node("ff_b/CLK", NodeKind::Clock);
        g.add_edge(root, shared, EdgeKind::Net, 0.50);
        g.add_edge(shared, ff_a, EdgeKind::Net, 0.10);
        g.add_edge(shared, ff_b, EdgeKind::Net, 0.10);
        g.finalize();
        let arr = clock_arrivals(&g, root).unwrap();
        // Common path delay is shared's arrival = 0.50.
        // Setup-check OCV: late = 1.10 (10% slow), early = 0.90 (10% fast).
        // Credit = 0.50 × (1.10 − 0.90) = 0.10.
        let credit = cppr_credit(&g, root, &arr, ff_a, ff_b, 1.10, 0.90);
        assert!((credit - 0.10).abs() < 1e-9);
    }
}
