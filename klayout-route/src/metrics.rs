//! Routing quality metrics.
//!
//! After a routing batch completes, callers want to know: total wire
//! length, via count, per-layer breakdown, congestion summary. This
//! module computes those without coupling to any specific consumer.
//!
//! Metrics are computed from the abstract `RoutedNet` data — they
//! don't require the layout to be stylized into shapes. So you can
//! run a routing batch in CI and dump metrics without ever creating
//! a `Library`.

use crate::detailed::RoutedNet;
use crate::global::{CapacityGrid, GCellGrid};
use crate::multilayer::RouteSegment;
use smol_str::SmolStr;

#[derive(Default, Clone, Debug)]
pub struct RoutingMetrics {
    /// Total wire length in DBU across all nets.
    pub total_wire_length: i64,
    /// Per-layer wire length in DBU.
    pub per_layer_length: Vec<i64>,
    /// Total via count (single-cut + array vias counted as 1 each).
    pub total_vias: usize,
    /// Total cut count summed across all via arrays.
    pub total_via_cuts: u64,
    /// Per-net wire length, indexed by net order in the input.
    pub per_net_length: Vec<NetMetrics>,
}

#[derive(Clone, Debug)]
pub struct NetMetrics {
    pub net_name: SmolStr,
    pub wire_length: i64,
    pub via_count: usize,
    pub via_cut_count: u64,
}

pub fn compute_metrics(routed: &[RoutedNet], layer_count: usize) -> RoutingMetrics {
    let mut m = RoutingMetrics {
        per_layer_length: vec![0; layer_count],
        ..RoutingMetrics::default()
    };
    for net in routed {
        let mut nm = NetMetrics {
            net_name: net.net_name.clone(),
            wire_length: 0,
            via_count: 0,
            via_cut_count: 0,
        };
        for seg in &net.segments {
            match seg {
                RouteSegment::Wire { layer_idx, points } => {
                    let len: i64 = points
                        .windows(2)
                        .map(|w| (w[1].x - w[0].x).abs() + (w[1].y - w[0].y).abs())
                        .sum();
                    nm.wire_length += len;
                    if *layer_idx < m.per_layer_length.len() {
                        m.per_layer_length[*layer_idx] += len;
                    }
                }
                RouteSegment::Via { cut_count, .. } => {
                    nm.via_count += 1;
                    nm.via_cut_count += *cut_count as u64;
                }
            }
        }
        m.total_wire_length += nm.wire_length;
        m.total_vias += nm.via_count;
        m.total_via_cuts += nm.via_cut_count;
        m.per_net_length.push(nm);
    }
    m
}

#[derive(Clone, Debug)]
pub struct CongestionSummary {
    pub total_edges: usize,
    pub saturated_edges: usize,
    pub max_overflow: i32,
    pub mean_load_pct: f64,
}

/// Summarise congestion over a capacity grid: how many edges are
/// saturated, what's the worst overflow, what's the mean load %.
pub fn summarize_congestion(grid: &GCellGrid, cap: &CapacityGrid) -> CongestionSummary {
    let mut total = 0usize;
    let mut sat = 0usize;
    let mut max_over: i32 = 0;
    let mut load_sum: i64 = 0;
    let count = grid.cell_count();
    for i in 0..count {
        // East edge.
        if i % grid.n_cols as usize + 1 < grid.n_cols as usize {
            total += 1;
            let used = cap.east_used.get(i).copied().unwrap_or(0) as i32;
            let capv = cap.east_capacity.get(i).copied().unwrap_or(0) as i32;
            if capv > 0 {
                if used >= capv {
                    sat += 1;
                }
                if used - capv > max_over {
                    max_over = used - capv;
                }
                load_sum += (used as i64 * 100) / capv as i64;
            }
        }
        // North edge.
        if i / grid.n_cols as usize + 1 < grid.n_rows as usize {
            total += 1;
            let used = cap.north_used.get(i).copied().unwrap_or(0) as i32;
            let capv = cap.north_capacity.get(i).copied().unwrap_or(0) as i32;
            if capv > 0 {
                if used >= capv {
                    sat += 1;
                }
                if used - capv > max_over {
                    max_over = used - capv;
                }
                load_sum += (used as i64 * 100) / capv as i64;
            }
        }
    }
    let mean_load_pct = if total > 0 {
        load_sum as f64 / total as f64
    } else {
        0.0
    };
    CongestionSummary {
        total_edges: total,
        saturated_edges: sat,
        max_overflow: max_over,
        mean_load_pct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use klayout_core::Point;

    fn wire(layer: usize, pts: Vec<Point>) -> RouteSegment {
        RouteSegment::Wire {
            layer_idx: layer,
            points: pts,
        }
    }

    fn via(at: Point, from: usize, to: usize, cuts: u32) -> RouteSegment {
        RouteSegment::Via {
            at,
            from_layer: from,
            to_layer: to,
            cut_count: cuts,
        }
    }

    #[test]
    fn metrics_sum_wire_length_per_layer() {
        let nets = vec![RoutedNet {
            net_name: "n0".into(),
            segments: vec![
                wire(0, vec![Point::new(0, 0), Point::new(50, 0)]),
                via(Point::new(50, 0), 0, 1, 4),
                wire(1, vec![Point::new(50, 0), Point::new(50, 30)]),
            ],
        }];
        let m = compute_metrics(&nets, 2);
        assert_eq!(m.per_layer_length[0], 50);
        assert_eq!(m.per_layer_length[1], 30);
        assert_eq!(m.total_wire_length, 80);
        assert_eq!(m.total_vias, 1);
        assert_eq!(m.total_via_cuts, 4);
    }

    #[test]
    fn per_net_metrics_recorded() {
        let nets = vec![
            RoutedNet {
                net_name: "a".into(),
                segments: vec![wire(0, vec![Point::new(0, 0), Point::new(10, 0)])],
            },
            RoutedNet {
                net_name: "b".into(),
                segments: vec![wire(0, vec![Point::new(0, 0), Point::new(20, 0)])],
            },
        ];
        let m = compute_metrics(&nets, 1);
        assert_eq!(m.per_net_length.len(), 2);
        assert_eq!(m.per_net_length[0].wire_length, 10);
        assert_eq!(m.per_net_length[1].wire_length, 20);
    }

    #[test]
    fn empty_routed_yields_zero_metrics() {
        let m = compute_metrics(&[], 2);
        assert_eq!(m.total_wire_length, 0);
        assert_eq!(m.total_vias, 0);
    }
}
