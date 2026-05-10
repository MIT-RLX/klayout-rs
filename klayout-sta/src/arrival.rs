//! Arrival/required time propagation + slack.

use crate::graph::{NodeId, TimingGraph};
use crate::timing_model::{DelayContext, LinearDelayModel, TimingModel};
use crate::{Result, StaError};

/// Forward propagation: max-arrival-time per node.
///
/// Arrival at primary inputs / clock pins defaults to `0.0` (override
/// via `input_arrivals`). Any node not reachable from a starting input
/// stays at `f64::NEG_INFINITY`.
/// Run forward propagation with the linear delay model. Equivalent
/// to [`compute_arrivals_with_model(g, inputs, &LinearDelayModel)`].
pub fn compute_arrivals(
    g: &TimingGraph,
    input_arrivals: &[(NodeId, f64)],
) -> Result<Vec<f64>> {
    compute_arrivals_with_model(g, input_arrivals, &LinearDelayModel)
}

/// Forward propagation under an arbitrary timing model. Each edge's
/// delay is queried via `model.delay(...)` with the destination
/// node's accumulated load as `output_load`. `output_load` per node
/// is computed as the sum of capacitances on outgoing net edges
/// (modelled as `edge.delay` in absence of explicit cap data —
/// matching how SPEF post-processing supplies it).
///
/// **Slew is not propagated** by this entry point — every edge sees
/// `input_slew = 0.0`. For NLDM lookups whose delay depends on input
/// transition, use [`compute_arrivals_with_slew`] instead.
pub fn compute_arrivals_with_model(
    g: &TimingGraph,
    input_arrivals: &[(NodeId, f64)],
    model: &dyn TimingModel,
) -> Result<Vec<f64>> {
    let n = g.nodes.len();
    let mut arr = vec![f64::NEG_INFINITY; n];
    for n in g.primary_inputs() {
        arr[n.0 as usize] = 0.0;
    }
    for (n, t) in input_arrivals {
        arr[n.0 as usize] = *t;
    }
    let order = topo_sort(g)?;
    for nid in order {
        if arr[nid.0 as usize] == f64::NEG_INFINITY {
            continue;
        }
        let from_arr = arr[nid.0 as usize];
        for e in g.outgoing(nid) {
            // Accumulate fanout cap at the destination as proxy for
            // output_load; for proper STA, caller wires the SPEF/PEX
            // cap into edge.delay (Net edges carry the lumped cap).
            let load = g
                .outgoing(e.to)
                .filter(|f| matches!(f.kind, crate::graph::EdgeKind::Net))
                .map(|f| f.delay)
                .sum::<f64>();
            let ctx = DelayContext {
                edge: e,
                input_slew: 0.0,
                output_load: load,
            };
            let edge_delay = model.delay(ctx);
            let candidate = from_arr + edge_delay;
            let dst = e.to.0 as usize;
            if candidate > arr[dst] {
                arr[dst] = candidate;
            }
        }
    }
    Ok(arr)
}

/// Forward propagation that **also propagates per-node output slew**
/// (transition time) and feeds it as `input_slew` to downstream edges.
/// This is what NLDM delay tables actually consume — without it, the
/// `(input_slew, output_load)` lookup degenerates to `slew = 0`,
/// silently capping the slew axis at its lower bound and biasing
/// every cell delay toward the table's fastest entry.
///
/// Returns `(arrivals, output_slews)` where `output_slews[i]` is the
/// max-arrival slew at node `i`. Slew at primary inputs defaults to
/// the supplied `input_slews` (else `0.0`).
pub fn compute_arrivals_with_slew(
    g: &TimingGraph,
    input_arrivals: &[(NodeId, f64)],
    input_slews: &[(NodeId, f64)],
    model: &dyn TimingModel,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let n = g.nodes.len();
    let mut arr = vec![f64::NEG_INFINITY; n];
    let mut slew = vec![0.0f64; n];
    for n in g.primary_inputs() {
        arr[n.0 as usize] = 0.0;
    }
    for (node_id, t) in input_arrivals {
        arr[node_id.0 as usize] = *t;
    }
    for (node_id, s) in input_slews {
        slew[node_id.0 as usize] = *s;
    }
    let order = topo_sort(g)?;
    for nid in order {
        if arr[nid.0 as usize] == f64::NEG_INFINITY {
            continue;
        }
        let from_arr = arr[nid.0 as usize];
        let from_slew = slew[nid.0 as usize];
        for e in g.outgoing(nid) {
            // Receiver-pin load: lumped capacitance carried on the
            // outgoing net edges of the *destination* pin (one hop
            // ahead). Same convention as `compute_arrivals_with_model`.
            let load = g
                .outgoing(e.to)
                .filter(|f| matches!(f.kind, crate::graph::EdgeKind::Net))
                .map(|f| f.delay)
                .sum::<f64>();
            let ctx = DelayContext {
                edge: e,
                input_slew: from_slew,
                output_load: load,
            };
            let edge_delay = model.delay(ctx);
            let out_slew = model.transition(ctx);
            let candidate = from_arr + edge_delay;
            let dst = e.to.0 as usize;
            if candidate > arr[dst] {
                arr[dst] = candidate;
                slew[dst] = out_slew;
            }
        }
    }
    Ok((arr, slew))
}

/// Backward propagation: required time per node.
///
/// Required time at primary outputs is the supplied constraint
/// (typically the clock period). At each node we take the min over
/// outgoing edges of `(required_at_dst - edge_delay)`.
pub fn compute_required(
    g: &TimingGraph,
    output_requireds: &[(NodeId, f64)],
    default_required: f64,
) -> Result<Vec<f64>> {
    let n = g.nodes.len();
    let mut req = vec![f64::INFINITY; n];
    for n in g.primary_outputs() {
        req[n.0 as usize] = default_required;
    }
    for (n, t) in output_requireds {
        req[n.0 as usize] = *t;
    }
    let mut order = topo_sort(g)?;
    order.reverse();
    for nid in order {
        if req[nid.0 as usize] == f64::INFINITY {
            // Compute from outgoing edges.
            let mut best = f64::INFINITY;
            for e in g.outgoing(nid) {
                let r_dst = req[e.to.0 as usize];
                if r_dst.is_finite() {
                    let candidate = r_dst - e.delay;
                    if candidate < best {
                        best = candidate;
                    }
                }
            }
            req[nid.0 as usize] = best;
        }
    }
    Ok(req)
}

/// Slack = required − arrival per node. Negative slack on a primary
/// output (or sequential D-pin) is a setup violation.
pub fn compute_slacks(arrivals: &[f64], requireds: &[f64]) -> Vec<f64> {
    arrivals
        .iter()
        .zip(requireds.iter())
        .map(|(a, r)| r - a)
        .collect()
}

/// Kahn-style topo sort over the timing graph. Returns nodes in
/// dependency order (inputs first, outputs last). Errors on cycle.
pub fn topo_sort(g: &TimingGraph) -> Result<Vec<NodeId>> {
    let n = g.nodes.len();
    let mut indeg = vec![0usize; n];
    for e in &g.edges {
        indeg[e.to.0 as usize] += 1;
    }
    let mut queue: Vec<NodeId> = (0..n)
        .filter(|i| indeg[*i] == 0)
        .map(|i| NodeId(i as u32))
        .collect();
    let mut order = Vec::with_capacity(n);
    while let Some(nid) = queue.pop() {
        order.push(nid);
        for e in g.outgoing(nid) {
            let dst = e.to.0 as usize;
            indeg[dst] -= 1;
            if indeg[dst] == 0 {
                queue.push(e.to);
            }
        }
    }
    if order.len() != n {
        return Err(StaError::CycleDetected(0));
    }
    Ok(order)
}
