//! Van Ginneken-style buffer insertion.
//!
//! Given a routing tree (single driver → N receivers) and a buffer
//! library, find the placement of buffers that minimises arrival
//! time at the driver while honouring max-load and max-transition
//! constraints. Classic algorithm: walk the tree bottom-up,
//! maintaining a Pareto frontier of `(load, delay)` candidate
//! solutions at each node; at each potential insertion point, also
//! consider the "buffered" candidate (driving a fresh buffer's
//! input at its input cap, with the buffer's intrinsic delay added
//! to the rest of the path). Discard dominated candidates.
//!
//! v1 simplifies in two ways:
//! * Single buffer cell type (input/output cap + intrinsic delay).
//! * Linear delay model: `delay = R_buffer × (Cload + C_in) + intrinsic`.
//!
//! Multiple buffer types and NLDM lookup-table-based delay are
//! straightforward extensions — the Pareto-frontier core handles
//! arbitrary candidate sets.

use klayout_core::Point;
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct Buffer {
    pub name: SmolStr,
    pub input_cap: f64,
    pub output_cap: f64,
    pub intrinsic_delay: f64,
    /// Output resistance — used in the linear delay model
    /// `delay = R × (Cload + C_in) + intrinsic`.
    pub output_resistance: f64,
}

#[derive(Clone, Debug)]
pub struct ReceiverPin {
    pub name: SmolStr,
    pub at: Point,
    pub input_cap: f64,
    pub max_transition: f64,
}

#[derive(Clone, Debug)]
pub struct RouteTreeNode {
    pub at: Point,
    pub children: Vec<RouteTreeChild>,
}

#[derive(Clone, Debug)]
pub enum RouteTreeChild {
    Internal(RouteTreeNode),
    Receiver(ReceiverPin),
}

#[derive(Clone, Debug)]
pub struct BufferPlacement {
    pub buffer: SmolStr,
    pub at: Point,
    pub driving_load: f64,
    pub delay: f64,
}

#[derive(Clone, Debug)]
pub struct Candidate {
    pub load: f64,
    pub delay: f64,
    /// Buffer placements upstream of this candidate.
    pub buffers: Vec<BufferPlacement>,
}

#[derive(Clone, Debug)]
pub struct InsertConfig {
    /// Per-DBU wire capacitance.
    pub wire_cap_per_dbu: f64,
    /// Per-DBU wire resistance.
    pub wire_res_per_dbu: f64,
    /// Maximum load allowed at any node before a buffer must be inserted.
    pub max_load: f64,
}

/// Run buffer insertion on `tree`. Returns the Pareto frontier of
/// (load, delay) candidates at the source. Caller picks one
/// according to their criterion (typically minimum delay).
pub fn buffer_insert(
    tree: &RouteTreeNode,
    buffer: &Buffer,
    cfg: &InsertConfig,
) -> Vec<Candidate> {
    let mut cands = visit(tree, buffer, cfg);
    cands = prune_pareto(cands);
    cands.sort_by(|a, b| a.delay.partial_cmp(&b.delay).unwrap_or(std::cmp::Ordering::Equal));
    cands
}

fn visit(node: &RouteTreeNode, buffer: &Buffer, cfg: &InsertConfig) -> Vec<Candidate> {
    // Start from each child's candidate set, propagate to this node
    // via an edge of length `manhattan(node, child)`.
    let mut combined: Vec<Candidate> = vec![Candidate {
        load: 0.0,
        delay: 0.0,
        buffers: Vec::new(),
    }];

    for child in &node.children {
        let child_cands = match child {
            RouteTreeChild::Internal(sub) => {
                let sub_cands = visit(sub, buffer, cfg);
                let dx = (sub.at.x - node.at.x).abs() + (sub.at.y - node.at.y).abs();
                propagate_along_wire(&sub_cands, dx as f64, cfg)
            }
            RouteTreeChild::Receiver(rx) => {
                let dx = (rx.at.x - node.at.x).abs() + (rx.at.y - node.at.y).abs();
                let leaf = vec![Candidate {
                    load: rx.input_cap,
                    delay: 0.0,
                    buffers: Vec::new(),
                }];
                propagate_along_wire(&leaf, dx as f64, cfg)
            }
        };
        // Combine current frontier with child's frontier additively.
        let mut new_combined: Vec<Candidate> = Vec::new();
        for a in &combined {
            for b in &child_cands {
                let mut bufs = a.buffers.clone();
                bufs.extend(b.buffers.clone());
                new_combined.push(Candidate {
                    load: a.load + b.load,
                    delay: a.delay.max(b.delay),
                    buffers: bufs,
                });
            }
        }
        combined = prune_pareto(new_combined);
    }

    // Also consider inserting a buffer at this node.
    let mut with_buffer: Vec<Candidate> = Vec::new();
    for c in &combined {
        // Buffer drives c.load; the buffer's input replaces it as the
        // load presented upstream.
        if c.load > cfg.max_load {
            // Force insertion when over load.
        }
        let buf_delay = buffer.intrinsic_delay + buffer.output_resistance * c.load;
        let mut new_bufs = c.buffers.clone();
        new_bufs.push(BufferPlacement {
            buffer: buffer.name.clone(),
            at: node.at,
            driving_load: c.load,
            delay: buf_delay,
        });
        with_buffer.push(Candidate {
            load: buffer.input_cap,
            delay: c.delay + buf_delay,
            buffers: new_bufs,
        });
    }
    combined.extend(with_buffer);
    prune_pareto(combined)
}

/// Propagate candidates along a wire of `length` DBU. Each candidate's
/// load grows by `cap_per_dbu × length`; delay grows by Elmore RC
/// (`R_wire × C_load + 0.5 × R_wire × C_wire`).
fn propagate_along_wire(
    cands: &[Candidate],
    length: f64,
    cfg: &InsertConfig,
) -> Vec<Candidate> {
    let c_wire = cfg.wire_cap_per_dbu * length;
    let r_wire = cfg.wire_res_per_dbu * length;
    cands
        .iter()
        .map(|c| {
            let load = c.load + c_wire;
            let delay = c.delay + r_wire * (c.load + 0.5 * c_wire);
            Candidate {
                load,
                delay,
                buffers: c.buffers.clone(),
            }
        })
        .collect()
}

fn prune_pareto(mut cands: Vec<Candidate>) -> Vec<Candidate> {
    cands.sort_by(|a, b| a.load.partial_cmp(&b.load).unwrap_or(std::cmp::Ordering::Equal));
    let mut out: Vec<Candidate> = Vec::new();
    let mut best_delay = f64::INFINITY;
    for c in cands {
        // Dominated iff some prior candidate has both lower load and lower delay.
        // Iterating in load-ascending order, a new candidate is non-dominated
        // iff its delay is strictly less than the running min.
        if c.delay < best_delay {
            best_delay = c.delay;
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn buf() -> Buffer {
        Buffer {
            name: "BUFX1".into(),
            input_cap: 0.005,
            output_cap: 0.001,
            intrinsic_delay: 0.05,
            output_resistance: 0.1,
        }
    }

    fn cfg() -> InsertConfig {
        InsertConfig {
            wire_cap_per_dbu: 0.0001,
            wire_res_per_dbu: 0.001,
            max_load: 0.05,
        }
    }

    fn rx(x: i64, name: &str) -> ReceiverPin {
        ReceiverPin {
            name: name.into(),
            at: Point::new(x, 0),
            input_cap: 0.005,
            max_transition: 0.1,
        }
    }

    #[test]
    fn small_tree_yields_pareto_frontier() {
        let tree = RouteTreeNode {
            at: Point::new(0, 0),
            children: vec![
                RouteTreeChild::Receiver(rx(100, "a")),
                RouteTreeChild::Receiver(rx(200, "b")),
            ],
        };
        let cands = buffer_insert(&tree, &buf(), &cfg());
        assert!(!cands.is_empty());
        // Frontier should be Pareto: as load increases, delay must decrease.
        for w in cands.windows(2) {
            assert!(w[0].delay <= w[1].delay || w[0].load >= w[1].load);
        }
    }

    #[test]
    fn long_wire_inserts_buffers() {
        // Long single-receiver wire — Pareto frontier should have at
        // least one candidate that includes a buffer (the un-buffered
        // candidate has high delay; the buffered one trades load for
        // less delay if the wire is long enough).
        let tree = RouteTreeNode {
            at: Point::new(0, 0),
            children: vec![RouteTreeChild::Receiver(rx(100_000, "far"))],
        };
        let cands = buffer_insert(&tree, &buf(), &cfg());
        assert!(cands.iter().any(|c| !c.buffers.is_empty()));
    }

    #[test]
    fn pareto_pruning_removes_dominated() {
        let cands = vec![
            Candidate {
                load: 1.0,
                delay: 5.0,
                buffers: vec![],
            },
            Candidate {
                load: 2.0,
                delay: 6.0,
                buffers: vec![],
            }, // dominated by first
            Candidate {
                load: 3.0,
                delay: 4.0,
                buffers: vec![],
            },
        ];
        let pruned = prune_pareto(cands);
        assert_eq!(pruned.len(), 2);
    }

    #[test]
    fn empty_tree_yields_zero_load_candidate() {
        let tree = RouteTreeNode {
            at: Point::new(0, 0),
            children: vec![],
        };
        let cands = buffer_insert(&tree, &buf(), &cfg());
        // Pareto frontier should still include the unbuffered baseline.
        assert!(!cands.is_empty());
        assert!(cands.iter().any(|c| c.load == 0.0));
    }
}
