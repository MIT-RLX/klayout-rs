//! Advanced and Parametric On-Chip Variation (AOCV / POCV).
//!
//! Per-corner OCV (in `multi_corner.rs`) applies one multiplier to
//! every edge in the timing graph. AOCV refines this: derate factors
//! depend on the *depth* of logic from a clock — short paths get
//! larger derating (less averaging), long paths get smaller
//! derating. POCV goes further: each cell has a `(mean, sigma)`
//! pair, and derating combines them into a per-stage statistical
//! adjustment.
//!
//! v1 ships:
//! * [`AocvTable`] — depth → derate-factor lookup with linear
//!   interpolation between depth bins.
//! * [`apply_aocv`] — re-derate every edge by traversing the graph
//!   from primary inputs and tracking depth.
//! * [`PocvVariation`] — per-cell `(mean, sigma)` pair.
//! * [`apply_pocv`] — root-sum-square sigma propagation along the
//!   path; emit per-node `(mean, sigma)` pair.

use crate::graph::{NodeId, TimingGraph};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct AocvTable {
    /// Depth bins (sorted ascending). Index `i` → derate `factors[i]`.
    pub depth_bins: Vec<u32>,
    pub factors: Vec<f64>,
}

impl AocvTable {
    pub fn lookup(&self, depth: u32) -> f64 {
        let nb = self.depth_bins.len();
        let nf = self.factors.len();
        if nb == 0 || nf == 0 {
            return 1.0;
        }
        if depth <= self.depth_bins[0] {
            return self.factors[0];
        }
        // nb > 0 and nf > 0 here, so the indices below are in bounds.
        if depth >= self.depth_bins[nb - 1] {
            return self.factors[nf - 1];
        }
        for i in 0..self.depth_bins.len() - 1 {
            let d_lo = self.depth_bins[i];
            let d_hi = self.depth_bins[i + 1];
            if depth >= d_lo && depth <= d_hi {
                let t = (depth - d_lo) as f64 / (d_hi - d_lo).max(1) as f64;
                return self.factors[i] + t * (self.factors[i + 1] - self.factors[i]);
            }
        }
        1.0
    }
}

/// Compute the depth (number of cell-arc hops from any primary input)
/// for every node in the graph. Used as the AOCV-table key.
pub fn depths(graph: &TimingGraph) -> HashMap<NodeId, u32> {
    let mut out: HashMap<NodeId, u32> = HashMap::new();
    let order = match crate::arrival::topo_sort(graph) {
        Ok(o) => o,
        Err(_) => return out,
    };
    for n in graph.primary_inputs() {
        out.insert(n, 0);
    }
    for nid in order {
        let cur_depth = out.get(&nid).copied().unwrap_or(0);
        for e in graph.outgoing(nid) {
            let inc = if matches!(e.kind, crate::graph::EdgeKind::CellArc) {
                1
            } else {
                0
            };
            let dst_depth = cur_depth + inc;
            let prev = out.get(&e.to).copied().unwrap_or(0);
            if dst_depth > prev {
                out.insert(e.to, dst_depth);
            }
        }
    }
    out
}

/// Apply AOCV derating: each edge's delay is scaled by the derate
/// factor at the destination node's depth.
pub fn apply_aocv(graph: &TimingGraph, table: &AocvTable) -> TimingGraph {
    let depths = depths(graph);
    let mut out = graph.clone();
    for e in out.edges.iter_mut() {
        let depth = depths.get(&e.to).copied().unwrap_or(0);
        let factor = table.lookup(depth);
        e.delay *= factor;
    }
    out
}

#[derive(Clone, Debug)]
pub struct PocvVariation {
    /// Mean delay multiplier (typically 1.0).
    pub mean: f64,
    /// Standard deviation of the multiplier (typically 0.0–0.2).
    pub sigma: f64,
}

#[derive(Clone, Debug)]
pub struct PocvNodeStat {
    pub node: NodeId,
    /// Cumulative mean arrival.
    pub mean_arrival: f64,
    /// Cumulative standard deviation (RSS-propagated).
    pub sigma_arrival: f64,
}

/// Run POCV propagation: each edge's `delay` is treated as the
/// mean; each cell-arc edge contributes `sigma * delay` to a
/// running RSS of standard deviations. Returns per-node
/// `(mean_arrival, sigma_arrival)`.
pub fn apply_pocv(
    graph: &TimingGraph,
    per_edge_sigma: &HashMap<usize, f64>,
) -> Vec<PocvNodeStat> {
    let n = graph.nodes.len();
    let mut means = vec![f64::NEG_INFINITY; n];
    let mut sigmas2 = vec![0.0f64; n];
    for nid in graph.primary_inputs() {
        means[nid.0 as usize] = 0.0;
    }
    let order = match crate::arrival::topo_sort(graph) {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };
    for nid in order {
        let cur_mean = means[nid.0 as usize];
        if cur_mean == f64::NEG_INFINITY {
            continue;
        }
        let cur_s2 = sigmas2[nid.0 as usize];
        for (edge_idx, e) in graph.edges.iter().enumerate() {
            if e.from != nid {
                continue;
            }
            let edge_sigma = per_edge_sigma.get(&edge_idx).copied().unwrap_or(0.0);
            let dst_mean = cur_mean + e.delay;
            let dst_s2 = cur_s2 + edge_sigma * edge_sigma;
            let dst = e.to.0 as usize;
            // Take max-mean; combine sigmas RSS-style on the chosen path.
            if dst_mean > means[dst] {
                means[dst] = dst_mean;
                sigmas2[dst] = dst_s2;
            }
        }
    }
    graph
        .nodes
        .iter()
        .map(|n| PocvNodeStat {
            node: n.id,
            mean_arrival: means[n.id.0 as usize],
            sigma_arrival: sigmas2[n.id.0 as usize].sqrt(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{EdgeKind, NodeKind, TimingGraph};

    fn linear_chain() -> TimingGraph {
        let mut g = TimingGraph::new();
        let pi = g.add_node("pi", NodeKind::PrimaryInput);
        let m1 = g.add_node("m1", NodeKind::CellOutput);
        let m2 = g.add_node("m2", NodeKind::CellOutput);
        let m3 = g.add_node("m3", NodeKind::CellOutput);
        let po = g.add_node("po", NodeKind::PrimaryOutput);
        g.add_edge(pi, m1, EdgeKind::CellArc, 0.1);
        g.add_edge(m1, m2, EdgeKind::CellArc, 0.1);
        g.add_edge(m2, m3, EdgeKind::CellArc, 0.1);
        g.add_edge(m3, po, EdgeKind::Net, 0.05);
        g.finalize();
        g
    }

    #[test]
    fn aocv_lookup_interpolates() {
        let t = AocvTable {
            depth_bins: vec![1, 5, 10],
            factors: vec![1.20, 1.10, 1.05],
        };
        assert!((t.lookup(1) - 1.20).abs() < 1e-9);
        assert!((t.lookup(5) - 1.10).abs() < 1e-9);
        assert!((t.lookup(10) - 1.05).abs() < 1e-9);
        // Mid-bin interpolation.
        assert!((t.lookup(3) - 1.15).abs() < 1e-9);
    }

    #[test]
    fn aocv_clamps_outside_range() {
        let t = AocvTable {
            depth_bins: vec![1, 5],
            factors: vec![1.20, 1.10],
        };
        assert!((t.lookup(0) - 1.20).abs() < 1e-9);
        assert!((t.lookup(100) - 1.10).abs() < 1e-9);
    }

    #[test]
    fn depths_count_cell_arcs() {
        let g = linear_chain();
        let d = depths(&g);
        // PI at depth 0; each cell-arc hop adds 1; net arc is 0.
        assert_eq!(d[&g.primary_inputs().next().unwrap()], 0);
        let po = g.primary_outputs().next().unwrap();
        assert_eq!(d[&po], 3);
    }

    #[test]
    fn aocv_scales_per_depth() {
        let g = linear_chain();
        let table = AocvTable {
            depth_bins: vec![1, 3],
            factors: vec![1.2, 1.05],
        };
        let derated = apply_aocv(&g, &table);
        // First edge (depth 1 destination): scale 1.2.
        // Last edge (depth 3 destination): scale 1.05.
        let first = &derated.edges[0];
        let last = derated.edges.last().unwrap();
        assert!((first.delay - 0.1 * 1.2).abs() < 1e-9);
        // Last edge is the net arc; depth at PO is still 3 → 1.05.
        assert!((last.delay - 0.05 * 1.05).abs() < 1e-9);
    }

    #[test]
    fn pocv_propagates_rss_sigma() {
        let g = linear_chain();
        let mut sigmas: HashMap<usize, f64> = HashMap::new();
        // Each cell arc has sigma 0.02; net arc has sigma 0.01.
        for (i, e) in g.edges.iter().enumerate() {
            if matches!(e.kind, EdgeKind::CellArc) {
                sigmas.insert(i, 0.02);
            } else {
                sigmas.insert(i, 0.01);
            }
        }
        let stats = apply_pocv(&g, &sigmas);
        let po = g.primary_outputs().next().unwrap();
        let po_stat = stats.iter().find(|s| s.node == po).unwrap();
        // Mean arrival = 3 × 0.1 + 0.05 = 0.35.
        assert!((po_stat.mean_arrival - 0.35).abs() < 1e-9);
        // Sigma RSS = sqrt(3 × 0.02² + 0.01²) ≈ 0.0354.
        let expected = (3.0 * 0.0004 + 0.0001f64).sqrt();
        assert!((po_stat.sigma_arrival - expected).abs() < 1e-6);
    }
}
