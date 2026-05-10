//! Antenna ratio check on routed nets.
//!
//! Antenna effects: during fab, conductors on lower metals act as
//! charge collectors that can damage the gate oxide they're connected
//! to before being shielded by upper metals. The fix is to limit the
//! ratio of conductor area / gate area at every level until a higher
//! metal "jumper" diverts the path.
//!
//! v1 implements the *check*: walk each net's segments, accumulate
//! per-layer area before reaching an up-via, divide by the connected
//! gate's area, and flag if the ratio exceeds the per-layer
//! `antenna_ratio` threshold. The repair (jumper insertion via a
//! higher layer) is a v2 — the check is itself the bulk of antenna
//! signoff.

use crate::detailed::RoutedNet;
use crate::multilayer::RouteSegment;
use klayout_core::Point;
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct AntennaRules {
    /// Per-layer antenna ratio limit (metal area / gate area).
    /// Typical values: 400 for M1, 600 for M2, 800 for higher metals.
    pub max_ratio: Vec<f64>,
    /// Per-layer wire width used to compute area = length × width.
    pub wire_width: Vec<i64>,
}

#[derive(Clone, Debug)]
pub struct GatePin {
    /// Hierarchical name `instance/pin` for diagnostics.
    pub name: SmolStr,
    /// The gate's location in DBU.
    pub at: Point,
    /// Layer the pin lives on (typically the lowest metal, e.g. M1).
    pub layer: usize,
    /// Effective gate-oxide area in DBU². Used as denominator.
    pub gate_area: f64,
}

#[derive(Clone, Debug)]
pub struct AntennaViolation {
    pub net_name: SmolStr,
    pub gate: SmolStr,
    pub layer: usize,
    /// Cumulative metal area in DBU² that contributes to this gate.
    pub metal_area: f64,
    pub gate_area: f64,
    pub ratio: f64,
    pub limit: f64,
}

/// Check all gate-pins on `net` against the antenna rules. Returns
/// one [`AntennaViolation`] per (gate, layer) pair that exceeds the
/// ratio limit.
pub fn antenna_check(
    net: &RoutedNet,
    gates: &[GatePin],
    rules: &AntennaRules,
) -> Vec<AntennaViolation> {
    let mut out = Vec::new();
    for gate in gates {
        // For each gate, walk the net and accumulate area on each layer
        // *up to but not including* the layer above the gate's layer.
        // The natural shielding model: once we cross a via to a layer
        // above `start_layer`, we stop — that via is the antenna fix.
        let mut per_layer_area: Vec<f64> = vec![0.0; rules.max_ratio.len()];
        accumulate_metal_area(net, gate.layer, &mut per_layer_area, rules);
        for (l, area) in per_layer_area.iter().enumerate() {
            if l >= rules.max_ratio.len() || gate.gate_area <= 0.0 {
                continue;
            }
            let ratio = area / gate.gate_area;
            if ratio > rules.max_ratio[l] {
                out.push(AntennaViolation {
                    net_name: net.net_name.clone(),
                    gate: gate.name.clone(),
                    layer: l,
                    metal_area: *area,
                    gate_area: gate.gate_area,
                    ratio,
                    limit: rules.max_ratio[l],
                });
            }
        }
    }
    out
}

/// Repair antenna violations by inserting jumpers — break a long
/// offending wire into segments separated by an up-via to the next
/// layer + small higher-metal hop + down-via. This shields each
/// segment from the gate.
///
/// v1 strategy: for each violation on layer `l`, walk the wire
/// segments on `l` and insert a jumper (up-via to `l+1`, ~`pad` long
/// jog on `l+1`, down-via back to `l`) at the wire's midpoint. This
/// halves the antenna area; if still over the limit, repeat. Bounded
/// by `max_jumpers_per_violation` to prevent runaway insertion.
pub fn antenna_fix(
    net: RoutedNet,
    gates: &[GatePin],
    rules: &AntennaRules,
    pad: i64,
    max_jumpers_per_violation: u32,
) -> RoutedNet {
    let mut net = net;
    for _ in 0..max_jumpers_per_violation {
        let viols = antenna_check(&net, gates, rules);
        if viols.is_empty() {
            break;
        }
        // Address every violating layer in this iteration: insert one
        // jumper on the longest wire of each unique offending layer.
        let mut layers_done: std::collections::HashSet<usize> =
            std::collections::HashSet::new();
        let mut sorted_viols: Vec<&AntennaViolation> = viols.iter().collect();
        sorted_viols.sort_by(|a, b| {
            b.ratio
                .partial_cmp(&a.ratio)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut any_inserted = false;
        for v in sorted_viols {
            if !layers_done.insert(v.layer) {
                continue;
            }
            if insert_jumper(&mut net, v.layer, pad) {
                any_inserted = true;
            }
        }
        if !any_inserted {
            break;
        }
    }
    net
}

/// Find the longest wire on `layer` in `net` and split it with a
/// jumper. Returns `true` if a jumper was inserted.
fn insert_jumper(net: &mut RoutedNet, layer: usize, pad: i64) -> bool {
    // Find the segment index + wire-internal index that has the
    // longest single edge on `layer`.
    let mut best: Option<(usize, usize, i64)> = None; // (seg_idx, edge_idx, length)
    for (si, seg) in net.segments.iter().enumerate() {
        if let RouteSegment::Wire { layer_idx, points } = seg {
            if *layer_idx != layer {
                continue;
            }
            for (ei, w) in points.windows(2).enumerate() {
                let len = (w[1].x - w[0].x).abs() + (w[1].y - w[0].y).abs();
                match best {
                    None => best = Some((si, ei, len)),
                    Some((_, _, prev)) if len > prev => best = Some((si, ei, len)),
                    _ => {}
                }
            }
        }
    }
    let (si, ei, len) = match best {
        Some(x) if x.2 >= pad * 2 => x,
        _ => return false,
    };

    // Split the chosen wire at its midpoint.
    let (a, b, layer_idx) = match &net.segments[si] {
        RouteSegment::Wire { layer_idx, points } => (points[ei], points[ei + 1], *layer_idx),
        _ => return false,
    };
    let mid_x = (a.x + b.x) / 2;
    let mid_y = (a.y + b.y) / 2;
    let mid = Point::new(mid_x, mid_y);

    // Build the jumper: pre-segment, up-via, jump segment on layer+1,
    // down-via, post-segment.
    let dx = (b.x - a.x).signum() * pad;
    let dy = (b.y - a.y).signum() * pad;
    let jump_a = Point::new(mid_x - dx / 2, mid_y - dy / 2);
    let jump_b = Point::new(mid_x + dx / 2, mid_y + dy / 2);

    // Replace the original wire with the pre + post halves.
    let pre = RouteSegment::Wire {
        layer_idx,
        points: vec![a, jump_a],
    };
    let post = RouteSegment::Wire {
        layer_idx,
        points: vec![jump_b, b],
    };
    let up = RouteSegment::Via {
        at: jump_a,
        from_layer: layer_idx,
        to_layer: layer_idx + 1,
        cut_count: 1,
    };
    let jump = RouteSegment::Wire {
        layer_idx: layer_idx + 1,
        points: vec![jump_a, jump_b],
    };
    let down = RouteSegment::Via {
        at: jump_b,
        from_layer: layer_idx + 1,
        to_layer: layer_idx,
        cut_count: 1,
    };

    // Splice into the segments vector: replace segments[si] with five new segments.
    net.segments
        .splice(si..=si, [pre, up, jump, down, post]);

    let _ = mid; // silence unused warning if any
    let _ = len;
    true
}

/// Sum metal area contributed by `net` per layer, from the gate's
/// starting layer up. Stops contributing on a layer once a higher-
/// layer via is encountered (a "jumper" shields any further metal on
/// that layer from the gate).
fn accumulate_metal_area(
    net: &RoutedNet,
    start_layer: usize,
    per_layer: &mut [f64],
    rules: &AntennaRules,
) {
    // Track which layers have been "shielded" by reaching an up-via.
    let mut shielded = vec![false; per_layer.len()];
    for seg in &net.segments {
        match seg {
            RouteSegment::Wire { layer_idx, points } => {
                if *layer_idx < start_layer || *layer_idx >= per_layer.len() {
                    continue;
                }
                if shielded[*layer_idx] {
                    continue;
                }
                let length: i64 = points
                    .windows(2)
                    .map(|w| (w[1].x - w[0].x).abs() + (w[1].y - w[0].y).abs())
                    .sum();
                let w = rules.wire_width.get(*layer_idx).copied().unwrap_or(1);
                per_layer[*layer_idx] += (length * w) as f64;
            }
            RouteSegment::Via {
                from_layer,
                to_layer,
                ..
            } => {
                // A via *up* from the start layer (or above) shields
                // the from-layer for the rest of the walk.
                if *to_layer > *from_layer && *from_layer >= start_layer {
                    if let Some(s) = shielded.get_mut(*from_layer) {
                        *s = true;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detailed::RoutedNet;
    use crate::multilayer::RouteSegment;
    use klayout_core::Point;

    fn rules(max_m1: f64, w: i64) -> AntennaRules {
        AntennaRules {
            max_ratio: vec![max_m1, 600.0, 800.0],
            wire_width: vec![w, w, w],
        }
    }

    fn wire(layer: usize, pts: Vec<Point>) -> RouteSegment {
        RouteSegment::Wire {
            layer_idx: layer,
            points: pts,
        }
    }

    fn via(at: Point, from: usize, to: usize) -> RouteSegment {
        RouteSegment::Via {
            at,
            from_layer: from,
            to_layer: to,
            cut_count: 1,
        }
    }

    #[test]
    fn small_net_passes() {
        let net = RoutedNet {
            net_name: "n0".into(),
            segments: vec![wire(0, vec![Point::new(0, 0), Point::new(10, 0)])],
        };
        let gates = vec![GatePin {
            name: "u0/A".into(),
            at: Point::new(0, 0),
            layer: 0,
            gate_area: 4.0,
        }];
        let viols = antenna_check(&net, &gates, &rules(100.0, 2));
        assert!(viols.is_empty());
    }

    #[test]
    fn long_thin_wire_violates() {
        // 10000 DBU long × 2 wide = 20000 DBU². Gate area 4 → ratio 5000.
        // M1 limit 400 → violation.
        let net = RoutedNet {
            net_name: "n0".into(),
            segments: vec![wire(0, vec![Point::new(0, 0), Point::new(10000, 0)])],
        };
        let gates = vec![GatePin {
            name: "u0/A".into(),
            at: Point::new(0, 0),
            layer: 0,
            gate_area: 4.0,
        }];
        let viols = antenna_check(&net, &gates, &rules(400.0, 2));
        assert_eq!(viols.len(), 1);
        assert_eq!(viols[0].layer, 0);
        assert!(viols[0].ratio > 400.0);
    }

    #[test]
    fn jumper_via_shields_subsequent_metal() {
        // Long M1 split by a via to M2 partway through. The portion
        // *after* the via is on M1 again, but the M1-then-up-via
        // shielding rule stops M1 area accumulation at the via.
        // (We model it as "once we go up from M1, M1 is shielded for
        // the rest of the walk.")
        let net = RoutedNet {
            net_name: "n0".into(),
            segments: vec![
                wire(0, vec![Point::new(0, 0), Point::new(100, 0)]),
                via(Point::new(100, 0), 0, 1),
                wire(0, vec![Point::new(100, 0), Point::new(20000, 0)]),
            ],
        };
        let gates = vec![GatePin {
            name: "u0/A".into(),
            at: Point::new(0, 0),
            layer: 0,
            gate_area: 4.0,
        }];
        let viols = antenna_check(&net, &gates, &rules(400.0, 2));
        // M1 area = 100×2 = 200 (only the pre-via wire counted).
        // Ratio 200/4 = 50 < 400 → pass.
        assert!(
            viols.is_empty(),
            "jumper via should shield the trailing M1; got {viols:?}"
        );
    }

    #[test]
    fn antenna_fix_reduces_violation() {
        // Long M1 wire that violates antenna ratio.
        let net = RoutedNet {
            net_name: "n0".into(),
            segments: vec![wire(0, vec![Point::new(0, 0), Point::new(20000, 0)])],
        };
        let gates = vec![GatePin {
            name: "u0/A".into(),
            at: Point::new(0, 0),
            layer: 0,
            gate_area: 4.0,
        }];
        let r = rules(400.0, 2);
        let viols_before = antenna_check(&net, &gates, &r);
        assert!(!viols_before.is_empty());
        let fixed = antenna_fix(net, &gates, &r, 50, 5);
        let viols_after = antenna_check(&fixed, &gates, &r);
        assert!(
            viols_after.len() < viols_before.len() || viols_after.iter().all(|v| v.ratio < viols_before[0].ratio),
            "antenna_fix should reduce violations"
        );
    }

    #[test]
    fn multi_layer_jumper_repair() {
        // Net with violations on M1 AND M2 simultaneously.
        let net = RoutedNet {
            net_name: "n".into(),
            segments: vec![
                wire(0, vec![Point::new(0, 0), Point::new(20000, 0)]),
                via(Point::new(20000, 0), 0, 1),
                wire(1, vec![Point::new(20000, 0), Point::new(40000, 0)]),
            ],
        };
        let gates = vec![GatePin {
            name: "g".into(),
            at: Point::new(0, 0),
            layer: 0,
            gate_area: 4.0,
        }];
        let r = AntennaRules {
            max_ratio: vec![400.0, 600.0, 800.0],
            wire_width: vec![2, 2, 2],
        };
        let viols_before = antenna_check(&net, &gates, &r);
        // Multiple layers violating before fix.
        assert!(viols_before.iter().any(|v| v.layer == 0));
        assert!(viols_before.iter().any(|v| v.layer == 1));
        let fixed = antenna_fix(net, &gates, &r, 100, 5);
        let viols_after = antenna_check(&fixed, &gates, &r);
        // Each layer's ratio is at least halved — multi-jumper repair
        // brings every violating layer down.
        let max_before = viols_before
            .iter()
            .map(|v| v.ratio)
            .fold(0.0f64, f64::max);
        let max_after = viols_after
            .iter()
            .map(|v| v.ratio)
            .fold(0.0f64, f64::max);
        assert!(
            max_after < max_before / 2.0,
            "expected halved ratio, got {max_before} → {max_after}"
        );
    }

    #[test]
    fn higher_layer_violation_reported_separately() {
        // Tiny M1 (passes) but huge M2 wire (violates M2 limit 600).
        let net = RoutedNet {
            net_name: "n0".into(),
            segments: vec![
                wire(0, vec![Point::new(0, 0), Point::new(10, 0)]),
                via(Point::new(10, 0), 0, 1),
                wire(1, vec![Point::new(10, 0), Point::new(20000, 0)]),
            ],
        };
        let gates = vec![GatePin {
            name: "u0/A".into(),
            at: Point::new(0, 0),
            layer: 0,
            gate_area: 4.0,
        }];
        let viols = antenna_check(&net, &gates, &rules(400.0, 2));
        // Should violate on M2 (layer 1), not M1.
        assert!(viols.iter().any(|v| v.layer == 1));
    }
}
