//! SPEF back-annotation onto the timing graph.
//!
//! Closes the loop between routing-derived parasitics and STA: given
//! a list of per-net `(R, C, pins)` triples extracted from SPEF (or
//! produced by [`klayout_connect::pex`]), update every `EdgeKind::Net`
//! edge in the timing graph with the corresponding interconnect delay.
//!
//! ## Delay model
//!
//! v1 uses the **lumped Elmore approximation**:
//!
//! ```text
//!     d_net = R · C
//! ```
//!
//! where `R` is the total path resistance and `C` is the total net
//! capacitance (driver pin cap + all sink pin caps + interconnect
//! ground caps). All sink-side `Net` edges of a given net receive the
//! same delay value — this is the textbook conservative
//! approximation for a fanout-1 wire and within ~15% of a true
//! Elmore tree-walk for typical fanout-≤4 nets.
//!
//! ## What's not in v1
//!
//! * **Per-sink Elmore delay** that distributes `R · C` across the
//!   detailed RC tree topology. Real STA tools build per-sink
//!   transfer functions; this is the right next upgrade and is what
//!   distinguishes a sign-off-grade flow from a v1.
//! * **Slew degradation** through the net (Bakoglu's slew model).
//!   v1 forwards the driver's input slew unchanged; net-induced
//!   slew degradation is on the same upgrade path.
//! * **CCS-driven receiver-side voltage waveform**.
//!
//! Each upgrade plugs into the same back-annotation API surface, so
//! callers don't need to refactor when these arrive.

use crate::graph::{EdgeKind, TimingGraph};
use smol_str::SmolStr;
use std::collections::{HashMap, HashSet};

/// Generic per-net parasitic record. Constructed from SPEF, PEX, or
/// any other source that produces `(R_total, C_total, pin_set)` tuples.
#[derive(Clone, Debug)]
pub struct NetParasitic {
    pub net_name: SmolStr,
    pub total_resistance: f64,
    pub total_capacitance: f64,
    /// Pin names of the form `instance/pin`. The first entry is
    /// conventionally the driver; remaining entries are sinks. Order
    /// doesn't matter for the lumped-delay approximation but tools
    /// that upgrade to per-sink Elmore will rely on it.
    pub pin_names: Vec<SmolStr>,
}

/// Back-annotate a list of per-net parasitics onto `graph`. Returns
/// `(edges_updated, nets_skipped)` for diagnostics: a high
/// `nets_skipped` count usually means the SPEF and the timing graph
/// were built with different instance-name conventions (e.g. `/`
/// vs `.` divider).
pub fn back_annotate_spef(
    nets: &[NetParasitic],
    graph: &mut TimingGraph,
) -> (usize, usize) {
    // Index Net edges by their `(from_name, to_name)` pair.
    let mut net_edges: HashMap<(SmolStr, SmolStr), usize> = HashMap::new();
    for (i, e) in graph.edges.iter().enumerate() {
        if !matches!(e.kind, EdgeKind::Net) {
            continue;
        }
        let from = graph.nodes[e.from.0 as usize].name.clone();
        let to = graph.nodes[e.to.0 as usize].name.clone();
        net_edges.insert((from, to), i);
    }

    let mut edges_updated = 0usize;
    let mut nets_skipped = 0usize;

    for net in nets {
        let delay = net.total_resistance * net.total_capacitance;
        let pin_set: HashSet<&SmolStr> = net.pin_names.iter().collect();
        let mut net_touched_any = false;

        // For every (driver, sink) pair where both pins are in this
        // net's pin set, update the Net edge.
        for driver in &pin_set {
            for sink in &pin_set {
                if driver == sink {
                    continue;
                }
                let key = ((*driver).clone(), (*sink).clone());
                if let Some(&idx) = net_edges.get(&key) {
                    graph.edges[idx].delay = delay;
                    edges_updated += 1;
                    net_touched_any = true;
                }
            }
        }
        if !net_touched_any {
            nets_skipped += 1;
        }
    }
    (edges_updated, nets_skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::NodeKind;

    #[test]
    fn empty_inputs_no_op() {
        let mut g = TimingGraph::new();
        let (n, s) = back_annotate_spef(&[], &mut g);
        assert_eq!(n, 0);
        assert_eq!(s, 0);
    }

    #[test]
    fn lumped_delay_applied_to_matching_net_edge() {
        let mut g = TimingGraph::new();
        let drv = g.add_node("u_drv/Y", NodeKind::CellOutput);
        let sink = g.add_node("u_sink/A", NodeKind::CellInput);
        g.add_edge(drv, sink, EdgeKind::Net, 0.0);
        g.finalize();

        let nets = vec![NetParasitic {
            net_name: "n0".into(),
            total_resistance: 100.0, // Ω
            total_capacitance: 0.001, // 1 fF in F
            pin_names: vec!["u_drv/Y".into(), "u_sink/A".into()],
        }];
        let (n, s) = back_annotate_spef(&nets, &mut g);
        assert_eq!(n, 1);
        assert_eq!(s, 0);
        // Delay = 100 * 0.001 = 0.1 (in whatever the input units imply).
        assert!((g.edges[0].delay - 0.1).abs() < 1e-9);
    }

    #[test]
    fn unmatched_net_increments_skip_count() {
        let mut g = TimingGraph::new();
        let drv = g.add_node("foo/Y", NodeKind::CellOutput);
        let sink = g.add_node("bar/A", NodeKind::CellInput);
        g.add_edge(drv, sink, EdgeKind::Net, 0.0);
        g.finalize();

        let nets = vec![NetParasitic {
            net_name: "n_other".into(),
            total_resistance: 10.0,
            total_capacitance: 0.01,
            pin_names: vec!["nope/A".into(), "nope/B".into()],
        }];
        let (n, s) = back_annotate_spef(&nets, &mut g);
        assert_eq!(n, 0);
        assert_eq!(s, 1);
    }

    #[test]
    fn cell_arc_edges_are_left_alone() {
        let mut g = TimingGraph::new();
        let a = g.add_node("u/A", NodeKind::CellInput);
        let y = g.add_node("u/Y", NodeKind::CellOutput);
        g.add_edge(a, y, EdgeKind::CellArc, 0.5); // existing cell delay
        g.finalize();

        let nets = vec![NetParasitic {
            net_name: "n".into(),
            total_resistance: 1.0,
            total_capacitance: 1.0,
            pin_names: vec!["u/A".into(), "u/Y".into()],
        }];
        back_annotate_spef(&nets, &mut g);
        assert_eq!(g.edges[0].delay, 0.5);
    }

    #[test]
    fn multi_sink_net_annotates_all_sinks() {
        let mut g = TimingGraph::new();
        let drv = g.add_node("d/Y", NodeKind::CellOutput);
        let s1 = g.add_node("s1/A", NodeKind::CellInput);
        let s2 = g.add_node("s2/A", NodeKind::CellInput);
        g.add_edge(drv, s1, EdgeKind::Net, 0.0);
        g.add_edge(drv, s2, EdgeKind::Net, 0.0);
        g.finalize();

        let nets = vec![NetParasitic {
            net_name: "n".into(),
            total_resistance: 50.0,
            total_capacitance: 0.002,
            pin_names: vec!["d/Y".into(), "s1/A".into(), "s2/A".into()],
        }];
        let (n, _) = back_annotate_spef(&nets, &mut g);
        assert_eq!(n, 2);
        // Both edges share the lumped delay.
        let expected = 50.0 * 0.002;
        for e in &g.edges {
            assert!((e.delay - expected).abs() < 1e-9);
        }
    }
}
