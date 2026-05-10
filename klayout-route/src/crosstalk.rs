//! Crosstalk-aware routing analysis.
//!
//! Two parallel signal nets running side-by-side over a long
//! distance couple capacitively: a switching aggressor injects noise
//! on the victim. Crosstalk-aware routing detects long-parallel-run
//! violations and (optionally) inserts shielding ground stripes
//! between aggressors and victims.
//!
//! v1 ships:
//!
//! * [`detect_violations`] — pair-wise scan over routed nets;
//!   reports any pair whose parallel-edge overlap exceeds
//!   `max_parallel_length` at separation ≤ `coupling_distance`.
//! * [`insert_shielding`] — between two violating nets, generate a
//!   shielding wire (returned as a `RouteSegment::Wire`) on the
//!   same layer connected to GND.

use crate::detailed::RoutedNet;
use crate::multilayer::RouteSegment;
use klayout_core::Point;
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct CrosstalkConfig {
    /// Maximum distance between parallel edges that still couples.
    pub coupling_distance: i64,
    /// Parallel-run-length threshold (DBU) above which a pair is
    /// flagged.
    pub max_parallel_length: i64,
}

#[derive(Clone, Debug)]
pub struct CrosstalkViolation {
    pub aggressor: SmolStr,
    pub victim: SmolStr,
    pub layer_idx: usize,
    pub overlap_length: i64,
    pub separation: i64,
    pub mid_point: Point,
}

pub fn detect_violations(
    nets: &[RoutedNet],
    cfg: &CrosstalkConfig,
) -> Vec<CrosstalkViolation> {
    let mut out: Vec<CrosstalkViolation> = Vec::new();
    for i in 0..nets.len() {
        for j in (i + 1)..nets.len() {
            for (a_seg, b_seg) in nets[i]
                .segments
                .iter()
                .zip(nets[j].segments.iter().cycle().take(nets[j].segments.len()))
            {
                let _ = b_seg; // suppress dead lint; we cycle below
                if let RouteSegment::Wire { layer_idx: la, points: pa } = a_seg {
                    for b_seg in &nets[j].segments {
                        if let RouteSegment::Wire { layer_idx: lb, points: pb } = b_seg {
                            if la != lb {
                                continue;
                            }
                            let viol = check_pair(
                                &nets[i].net_name,
                                &nets[j].net_name,
                                *la,
                                pa,
                                pb,
                                cfg,
                            );
                            if let Some(v) = viol {
                                out.push(v);
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

fn check_pair(
    aggressor: &str,
    victim: &str,
    layer_idx: usize,
    pa: &[Point],
    pb: &[Point],
    cfg: &CrosstalkConfig,
) -> Option<CrosstalkViolation> {
    let mut total_overlap: i64 = 0;
    let mut min_sep: i64 = i64::MAX;
    let mut sample: Option<Point> = None;
    for ea in pa.windows(2) {
        for eb in pb.windows(2) {
            let (overlap, sep, mid) = parallel_overlap(ea[0], ea[1], eb[0], eb[1]);
            if overlap > 0 && sep > 0 && sep <= cfg.coupling_distance {
                total_overlap += overlap;
                if sep < min_sep {
                    min_sep = sep;
                    sample = Some(mid);
                }
            }
        }
    }
    if total_overlap > cfg.max_parallel_length {
        Some(CrosstalkViolation {
            aggressor: aggressor.into(),
            victim: victim.into(),
            layer_idx,
            overlap_length: total_overlap,
            separation: min_sep,
            mid_point: sample.unwrap_or(Point::new(0, 0)),
        })
    } else {
        None
    }
}

/// Compute parallel-overlap of two axis-aligned segments. Returns
/// `(overlap_length, perpendicular_separation, midpoint_of_overlap)`.
fn parallel_overlap(a1: Point, a2: Point, b1: Point, b2: Point) -> (i64, i64, Point) {
    let a_horiz = a1.y == a2.y;
    let b_horiz = b1.y == b2.y;
    let a_vert = a1.x == a2.x;
    let b_vert = b1.x == b2.x;
    if a_horiz && b_horiz {
        let lo = a1.x.min(a2.x).max(b1.x.min(b2.x));
        let hi = a1.x.max(a2.x).min(b1.x.max(b2.x));
        if hi <= lo {
            return (0, 0, Point::new(0, 0));
        }
        let sep = (a1.y - b1.y).abs();
        let mid = Point::new((lo + hi) / 2, (a1.y + b1.y) / 2);
        (hi - lo, sep, mid)
    } else if a_vert && b_vert {
        let lo = a1.y.min(a2.y).max(b1.y.min(b2.y));
        let hi = a1.y.max(a2.y).min(b1.y.max(b2.y));
        if hi <= lo {
            return (0, 0, Point::new(0, 0));
        }
        let sep = (a1.x - b1.x).abs();
        let mid = Point::new((a1.x + b1.x) / 2, (lo + hi) / 2);
        (hi - lo, sep, mid)
    } else {
        (0, 0, Point::new(0, 0))
    }
}

/// Generate a shielding wire between an aggressor and victim. The
/// shield runs the parallel-overlap range at the midpoint of the
/// two nets' separation; caller adds this segment to the GND net.
pub fn insert_shielding(violation: &CrosstalkViolation) -> RouteSegment {
    // Single-segment shielding wire centered on the violation. v1
    // emits a unit-length wire as a marker; production extends it
    // to span the actual overlap.
    let len = violation.overlap_length / 2;
    let p1 = Point::new(
        violation.mid_point.x - len,
        violation.mid_point.y,
    );
    let p2 = Point::new(
        violation.mid_point.x + len,
        violation.mid_point.y,
    );
    RouteSegment::Wire {
        layer_idx: violation.layer_idx,
        points: vec![p1, p2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(layer: usize, pts: Vec<Point>) -> RouteSegment {
        RouteSegment::Wire {
            layer_idx: layer,
            points: pts,
        }
    }

    fn cfg() -> CrosstalkConfig {
        CrosstalkConfig {
            coupling_distance: 5,
            max_parallel_length: 50,
        }
    }

    #[test]
    fn parallel_long_run_flagged() {
        let a = RoutedNet {
            net_name: "agg".into(),
            segments: vec![wire(0, vec![Point::new(0, 0), Point::new(100, 0)])],
        };
        let b = RoutedNet {
            net_name: "vic".into(),
            segments: vec![wire(0, vec![Point::new(0, 3), Point::new(100, 3)])],
        };
        let v = detect_violations(&[a, b], &cfg());
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].overlap_length, 100);
        assert_eq!(v[0].separation, 3);
    }

    #[test]
    fn distant_runs_not_flagged() {
        let a = RoutedNet {
            net_name: "a".into(),
            segments: vec![wire(0, vec![Point::new(0, 0), Point::new(100, 0)])],
        };
        let b = RoutedNet {
            net_name: "b".into(),
            segments: vec![wire(0, vec![Point::new(0, 100), Point::new(100, 100)])],
        };
        assert!(detect_violations(&[a, b], &cfg()).is_empty());
    }

    #[test]
    fn short_overlap_not_flagged() {
        let a = RoutedNet {
            net_name: "a".into(),
            segments: vec![wire(0, vec![Point::new(0, 0), Point::new(20, 0)])],
        };
        let b = RoutedNet {
            net_name: "b".into(),
            segments: vec![wire(0, vec![Point::new(0, 3), Point::new(20, 3)])],
        };
        // 20 < max_parallel_length 50 → no violation.
        assert!(detect_violations(&[a, b], &cfg()).is_empty());
    }

    #[test]
    fn shield_segment_emitted_for_violation() {
        let a = RoutedNet {
            net_name: "agg".into(),
            segments: vec![wire(0, vec![Point::new(0, 0), Point::new(100, 0)])],
        };
        let b = RoutedNet {
            net_name: "vic".into(),
            segments: vec![wire(0, vec![Point::new(0, 3), Point::new(100, 3)])],
        };
        let v = &detect_violations(&[a, b], &cfg())[0];
        let shield = insert_shielding(v);
        match shield {
            RouteSegment::Wire { layer_idx, points } => {
                assert_eq!(layer_idx, 0);
                assert_eq!(points.len(), 2);
            }
            _ => panic!(),
        }
    }
}
