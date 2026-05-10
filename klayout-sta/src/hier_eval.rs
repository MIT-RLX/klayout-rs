//! Hierarchical STA composition.
//!
//! Production hierarchical STA characterises each cell's boundary
//! timing once — propagation delay from each input pin to each
//! output pin — and composes parents using only that abstract.
//! Internal cell topology is invisible at the parent level. Two
//! flavours:
//!
//! * **Black-box composition** — each child cell appears in the
//!   parent graph as a single node-pair (input → output) with a
//!   pre-computed `delay` edge. The parent never sees the child's
//!   internal nodes.
//! * **Extracted-timing composition** — each child cell exports a
//!   `(input_pin, output_pin) → delay` table; the parent's graph
//!   replaces each instance with edges drawn from that table.
//!
//! v1 implements the second flavour. Each [`CellTimingAbstract`]
//! captures one cell's pin-to-pin worst-case delays. A parent's
//! [`TimingGraph`] then references these abstracts via instances.
//!
//! Caching: identical cells (same `content_hash`) share the same
//! `CellTimingAbstract`, so a 1000-instance design pays per-cell
//! characterisation cost once.

use crate::arrival::{compute_arrivals, compute_required};
use crate::graph::{EdgeKind, NodeId, NodeKind, TimingGraph};
use smol_str::SmolStr;
use std::collections::HashMap;

/// Per-cell timing abstract: worst-case combinational delay from
/// each input pin to each output pin. `pin_names` map pin labels
/// (the parent's per-instance binding) to indices.
#[derive(Clone, Debug, Default)]
pub struct CellTimingAbstract {
    pub cell_name: SmolStr,
    pub input_pins: Vec<SmolStr>,
    pub output_pins: Vec<SmolStr>,
    /// `delays[i * output_pins.len() + j]` = worst-case delay from
    /// `input_pins[i]` to `output_pins[j]`.
    pub delays: Vec<f64>,
}

impl CellTimingAbstract {
    pub fn new(cell_name: impl Into<SmolStr>) -> Self {
        Self {
            cell_name: cell_name.into(),
            ..Self::default()
        }
    }

    pub fn delay_at(&self, input_pin: &str, output_pin: &str) -> Option<f64> {
        let i = self.input_pins.iter().position(|n| n == input_pin)?;
        let j = self.output_pins.iter().position(|n| n == output_pin)?;
        let cols = self.output_pins.len();
        self.delays.get(i * cols + j).copied()
    }

    pub fn set_delay(&mut self, input_pin: &str, output_pin: &str, d: f64) {
        let i = match self.input_pins.iter().position(|n| n == input_pin) {
            Some(i) => i,
            None => {
                self.input_pins.push(SmolStr::from(input_pin));
                self.input_pins.len() - 1
            }
        };
        let j = match self.output_pins.iter().position(|n| n == output_pin) {
            Some(j) => j,
            None => {
                self.output_pins.push(SmolStr::from(output_pin));
                self.output_pins.len() - 1
            }
        };
        let cols = self.output_pins.len();
        let idx = i * cols + j;
        if self.delays.len() != self.input_pins.len() * cols {
            self.delays.resize(self.input_pins.len() * cols, 0.0);
        }
        self.delays[idx] = d;
    }
}

/// Extract a [`CellTimingAbstract`] from a fully-populated cell-
/// internal `TimingGraph`. Inputs are nodes labelled
/// `(input)/<pin>` (matching our convention); outputs are
/// `(output)/<pin>`. Worst-case delay = max arrival at the output
/// minus the input's start time (assumed 0).
pub fn extract_abstract(graph: &TimingGraph, cell_name: &str) -> CellTimingAbstract {
    let mut abs_ = CellTimingAbstract::new(cell_name);
    let inputs: Vec<NodeId> = graph.primary_inputs().collect();
    let outputs: Vec<NodeId> = graph.primary_outputs().collect();
    if inputs.is_empty() || outputs.is_empty() {
        return abs_;
    }
    for &i_node in &inputs {
        // Run arrival propagation from this single input only.
        let init: Vec<(NodeId, f64)> = inputs
            .iter()
            .map(|&n| (n, if n == i_node { 0.0 } else { f64::NEG_INFINITY }))
            .collect();
        let arr = match compute_arrivals(graph, &init) {
            Ok(a) => a,
            Err(_) => continue,
        };
        let in_name = pin_label(graph.node(i_node).name.as_str());
        for &o_node in &outputs {
            let out_name = pin_label(graph.node(o_node).name.as_str());
            let d = arr[o_node.0 as usize];
            if d.is_finite() {
                abs_.set_delay(&in_name, &out_name, d);
            }
        }
    }
    abs_
}

fn pin_label(node_name: &str) -> String {
    // Strip "(input)/" or "(output)/" prefix conventions.
    if let Some(rest) = node_name.strip_prefix("(input)/") {
        return rest.to_string();
    }
    if let Some(rest) = node_name.strip_prefix("(output)/") {
        return rest.to_string();
    }
    node_name.to_string()
}

/// Hierarchical STA at a parent cell: instances of child cells are
/// modelled as edges in the parent's graph using each child's
/// abstract. The caller supplies an `instance_to_abstract` map and
/// per-instance pin-to-net bindings; we add the corresponding edges
/// and run `compute_arrivals` / `compute_required` at the parent
/// level only.
pub struct HierStaInputs<'a> {
    pub parent: &'a TimingGraph,
    /// One abstract per child instance, indexed in the same order as
    /// the parent's instances list.
    pub instance_abstracts: Vec<&'a CellTimingAbstract>,
    /// Per-instance pin-to-node binding: which parent node each
    /// child input/output pin maps to.
    pub instance_pin_bindings: Vec<HashMap<SmolStr, NodeId>>,
}

#[derive(Clone, Debug)]
pub struct HierStaResult {
    pub arrivals: Vec<f64>,
    pub requireds: Vec<f64>,
}

pub fn compose_hier_timing(
    inputs: &HierStaInputs<'_>,
    clock_period: f64,
) -> HierStaResult {
    let mut composed: TimingGraph = inputs.parent.clone();
    // Add per-instance pin-to-pin edges from the child abstract.
    for (abs_, binding) in inputs
        .instance_abstracts
        .iter()
        .zip(inputs.instance_pin_bindings.iter())
    {
        for (i, in_pin) in abs_.input_pins.iter().enumerate() {
            for (j, out_pin) in abs_.output_pins.iter().enumerate() {
                let cols = abs_.output_pins.len();
                let d = abs_.delays.get(i * cols + j).copied().unwrap_or(0.0);
                if !d.is_finite() {
                    continue;
                }
                let from_node = binding.get(in_pin);
                let to_node = binding.get(out_pin);
                if let (Some(&fr), Some(&to)) = (from_node, to_node) {
                    composed.add_edge(fr, to, EdgeKind::CellArc, d);
                }
            }
        }
    }
    composed.finalize();
    let arrivals = compute_arrivals(&composed, &[]).unwrap_or_default();
    let requireds = compute_required(&composed, &[], clock_period).unwrap_or_default();
    HierStaResult {
        arrivals,
        requireds,
    }
}

/// Convenience: count nodes touched at the parent level. Useful for
/// measuring the win of hierarchical composition over flattening.
pub fn parent_node_count(parent: &TimingGraph) -> usize {
    parent
        .nodes
        .iter()
        .filter(|n| {
            !matches!(
                n.kind,
                NodeKind::CellInput | NodeKind::CellOutput
            )
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeKind, NodeKind, TimingGraph};

    fn small_cell_graph() -> TimingGraph {
        let mut g = TimingGraph::new();
        let a = g.add_node("(input)/A", NodeKind::PrimaryInput);
        let b = g.add_node("(input)/B", NodeKind::PrimaryInput);
        let mid = g.add_node("u/Y", NodeKind::CellOutput);
        let y = g.add_node("(output)/Y", NodeKind::PrimaryOutput);
        g.add_edge(a, mid, EdgeKind::CellArc, 0.10);
        g.add_edge(b, mid, EdgeKind::CellArc, 0.15);
        g.add_edge(mid, y, EdgeKind::Net, 0.05);
        g.finalize();
        g
    }

    #[test]
    fn extracts_pin_to_pin_delays() {
        let g = small_cell_graph();
        let abs_ = extract_abstract(&g, "INV");
        // A → Y = 0.10 + 0.05 = 0.15.
        // B → Y = 0.15 + 0.05 = 0.20.
        let a_y = abs_.delay_at("A", "Y").unwrap();
        let b_y = abs_.delay_at("B", "Y").unwrap();
        assert!((a_y - 0.15).abs() < 1e-9);
        assert!((b_y - 0.20).abs() < 1e-9);
    }

    #[test]
    fn compose_hier_uses_abstract_for_instance() {
        // Parent graph: PI → instance/A → instance/Y → PO.
        // Instance is one child cell with A→Y delay 0.20.
        let mut parent = TimingGraph::new();
        let pi = parent.add_node("(input)/I", NodeKind::PrimaryInput);
        let ia = parent.add_node("u0/A", NodeKind::CellInput);
        let iy = parent.add_node("u0/Y", NodeKind::CellOutput);
        let po = parent.add_node("(output)/O", NodeKind::PrimaryOutput);
        parent.add_edge(pi, ia, EdgeKind::Net, 0.05);
        parent.add_edge(iy, po, EdgeKind::Net, 0.05);
        parent.finalize();

        let mut abs_ = CellTimingAbstract::new("INV");
        abs_.set_delay("A", "Y", 0.20);

        let mut binding: HashMap<SmolStr, NodeId> = HashMap::new();
        binding.insert("A".into(), ia);
        binding.insert("Y".into(), iy);

        let inputs = HierStaInputs {
            parent: &parent,
            instance_abstracts: vec![&abs_],
            instance_pin_bindings: vec![binding],
        };
        let result = compose_hier_timing(&inputs, 1.0);
        // Arrival at PO = 0.05 (net) + 0.20 (instance abstract) + 0.05 (net).
        let po_arrival = result.arrivals[po.0 as usize];
        assert!((po_arrival - 0.30).abs() < 1e-6, "got {po_arrival}");
    }

    #[test]
    fn parent_node_count_excludes_cell_pins() {
        let mut g = TimingGraph::new();
        g.add_node("(input)/A", NodeKind::PrimaryInput);
        g.add_node("u/A", NodeKind::CellInput);
        g.add_node("u/Y", NodeKind::CellOutput);
        g.add_node("(output)/Y", NodeKind::PrimaryOutput);
        g.finalize();
        // 4 nodes total; 2 are cell pins → expect count 2.
        assert_eq!(parent_node_count(&g), 2);
    }
}
