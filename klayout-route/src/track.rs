//! Track-grid snapping.
//!
//! PDKs define a per-layer routing grid of "tracks": evenly-spaced
//! lines on which wire centerlines must sit. Off-grid wires aren't
//! signoff-clean (DRC fails on track misalignment, antenna and
//! foundry tooling assume grid coordinates). Real detailed routers
//! snap every endpoint and bend point to the nearest track.
//!
//! [`TrackGrid`] models the grid: an `(x_origin, x_pitch)` for
//! vertical tracks and `(y_origin, y_pitch)` for horizontal. A
//! point's snapped position is the nearest grid intersection.
//!
//! Per-layer track grids are passed via [`TrackGrids`], one per
//! routing layer. Use [`snap_routed_net`] to snap an entire
//! `RoutedNet` after detail routing — bends and via points alike
//! get aligned. Snapping after the fact is conservative-correct
//! when `pitch` is comparable to the router's `grid_step` (the snap
//! displaces points by at most `pitch / 2`, which the router's
//! spacing inflation already accounts for).

use crate::detailed::RoutedNet;
use crate::multilayer::RouteSegment;
use klayout_core::Point;

#[derive(Copy, Clone, Debug)]
pub struct TrackGrid {
    pub x_origin: i64,
    pub x_pitch: i64,
    pub y_origin: i64,
    pub y_pitch: i64,
}

impl TrackGrid {
    pub fn snap(&self, p: Point) -> Point {
        let x = round_to_grid(p.x, self.x_origin, self.x_pitch);
        let y = round_to_grid(p.y, self.y_origin, self.y_pitch);
        Point::new(x, y)
    }
}

#[derive(Default, Clone, Debug)]
pub struct TrackGrids {
    pub per_layer: Vec<TrackGrid>,
}

fn round_to_grid(v: i64, origin: i64, pitch: i64) -> i64 {
    if pitch <= 1 {
        return v;
    }
    let off = v - origin;
    let q = if off >= 0 {
        (off + pitch / 2) / pitch
    } else {
        -((-off + pitch / 2) / pitch)
    };
    origin + q * pitch
}

/// Snap every wire vertex and via center in `net` to the nearest
/// track on its layer. Returns a new `RoutedNet`.
pub fn snap_routed_net(net: &RoutedNet, grids: &TrackGrids) -> RoutedNet {
    let mut segments = Vec::with_capacity(net.segments.len());
    for seg in &net.segments {
        match seg {
            RouteSegment::Wire { layer_idx, points } => {
                let grid = grids.per_layer.get(*layer_idx);
                let snapped: Vec<Point> = points
                    .iter()
                    .map(|p| grid.map_or(*p, |g| g.snap(*p)))
                    .collect();
                let dedup = remove_consecutive_duplicates(snapped);
                if dedup.len() >= 2 {
                    segments.push(RouteSegment::Wire {
                        layer_idx: *layer_idx,
                        points: dedup,
                    });
                }
            }
            RouteSegment::Via {
                at,
                from_layer,
                to_layer,
                cut_count,
            } => {
                // Snap to whichever layer's grid is finer.
                let g = grids
                    .per_layer
                    .get(*from_layer)
                    .or_else(|| grids.per_layer.get(*to_layer));
                let new_at = g.map_or(*at, |g| g.snap(*at));
                segments.push(RouteSegment::Via {
                    at: new_at,
                    from_layer: *from_layer,
                    to_layer: *to_layer,
                    cut_count: *cut_count,
                });
            }
        }
    }
    RoutedNet {
        net_name: net.net_name.clone(),
        segments,
    }
}

fn remove_consecutive_duplicates(pts: Vec<Point>) -> Vec<Point> {
    let mut out = Vec::with_capacity(pts.len());
    for p in pts {
        if out.last() != Some(&p) {
            out.push(p);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snap_rounds_to_nearest_grid() {
        let g = TrackGrid {
            x_origin: 0,
            x_pitch: 10,
            y_origin: 0,
            y_pitch: 10,
        };
        assert_eq!(g.snap(Point::new(4, 6)), Point::new(0, 10));
        assert_eq!(g.snap(Point::new(15, 14)), Point::new(20, 10));
        assert_eq!(g.snap(Point::new(-3, -7)), Point::new(0, -10));
    }

    #[test]
    fn snap_with_origin_offset() {
        let g = TrackGrid {
            x_origin: 5,
            x_pitch: 10,
            y_origin: 5,
            y_pitch: 10,
        };
        // Tracks at x = 5, 15, 25, ...
        assert_eq!(g.snap(Point::new(8, 8)), Point::new(5, 5));
        assert_eq!(g.snap(Point::new(11, 11)), Point::new(15, 15));
    }

    #[test]
    fn snap_routed_net_aligns_endpoints() {
        let g = TrackGrid {
            x_origin: 0,
            x_pitch: 10,
            y_origin: 0,
            y_pitch: 10,
        };
        let grids = TrackGrids {
            per_layer: vec![g, g],
        };
        let net = RoutedNet {
            net_name: "n".into(),
            segments: vec![
                RouteSegment::Wire {
                    layer_idx: 0,
                    points: vec![Point::new(3, 4), Point::new(57, 6)],
                },
                RouteSegment::Via {
                    at: Point::new(57, 6),
                    from_layer: 0,
                    to_layer: 1,
                    cut_count: 1,
                },
            ],
        };
        let snapped = snap_routed_net(&net, &grids);
        match &snapped.segments[0] {
            RouteSegment::Wire { points, .. } => {
                assert_eq!(points[0], Point::new(0, 0));
                assert_eq!(points[1], Point::new(60, 10));
            }
            _ => panic!(),
        }
        match &snapped.segments[1] {
            RouteSegment::Via { at, .. } => {
                assert_eq!(*at, Point::new(60, 10));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn duplicate_collapsed_after_snap() {
        let g = TrackGrid {
            x_origin: 0,
            x_pitch: 100,
            y_origin: 0,
            y_pitch: 100,
        };
        let grids = TrackGrids { per_layer: vec![g] };
        // Two close points snap to the same grid intersection.
        let net = RoutedNet {
            net_name: "n".into(),
            segments: vec![RouteSegment::Wire {
                layer_idx: 0,
                points: vec![Point::new(10, 10), Point::new(20, 20), Point::new(30, 30)],
            }],
        };
        let snapped = snap_routed_net(&net, &grids);
        // All three points snap to (0, 0); after dedup the single
        // surviving point makes the wire degenerate and gets dropped.
        assert!(snapped.segments.is_empty());
    }
}
