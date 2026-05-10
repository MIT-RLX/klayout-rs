//! Pin assignment optimisation for block-level routing.
//!
//! When a block has *movable* pins (its boundary pins can sit
//! anywhere along the perimeter), the choice of pin location
//! significantly affects routing length. Pin-opt picks each pin's
//! position to minimise the total HPWL of its connected nets.
//!
//! Algorithm: per-pin, compute the centroid of the pin's connected
//! external-net positions; project onto the block's allowed pin
//! sides; snap to a track grid; resolve collisions left-to-right.
//!
//! v1 supports a rectangular block boundary with 4 sides ({N, S, E,
//! W}) — most digital blocks. Each pin's allowed sides are caller-
//! provided.

use klayout_core::{Bbox, Point};
use smol_str::SmolStr;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PinSide {
    North,
    South,
    East,
    West,
}

#[derive(Clone, Debug)]
pub struct PinReq {
    pub name: SmolStr,
    /// External-net pin positions this block pin connects to. Their
    /// centroid drives placement.
    pub net_pins: Vec<Point>,
    /// Sides this pin is allowed to sit on.
    pub allowed_sides: Vec<PinSide>,
}

#[derive(Clone, Debug)]
pub struct PinPlacement {
    pub name: SmolStr,
    pub side: PinSide,
    pub at: Point,
}

#[derive(Clone, Debug)]
pub struct PinOptConfig {
    pub block_bbox: Bbox,
    /// Track pitch along each side (DBU); 0 = no snap.
    pub track_pitch: i64,
    /// Minimum spacing between adjacent pins on the same side.
    pub pin_spacing: i64,
}

/// Place each pin on its best allowed side at its closest valid
/// track position. Resolves collisions by stepping along the side.
pub fn pin_opt(reqs: &[PinReq], cfg: &PinOptConfig) -> Vec<PinPlacement> {
    let mut out: Vec<PinPlacement> = Vec::with_capacity(reqs.len());
    let mut occupied_north: Vec<i64> = Vec::new();
    let mut occupied_south: Vec<i64> = Vec::new();
    let mut occupied_east: Vec<i64> = Vec::new();
    let mut occupied_west: Vec<i64> = Vec::new();

    for req in reqs {
        if req.net_pins.is_empty() || req.allowed_sides.is_empty() {
            continue;
        }
        let centroid = centroid(&req.net_pins);
        let Some((best_side, best_at)) = pick_side(centroid, cfg.block_bbox, &req.allowed_sides)
        else {
            continue;
        };
        let occ = match best_side {
            PinSide::North => &mut occupied_north,
            PinSide::South => &mut occupied_south,
            PinSide::East => &mut occupied_east,
            PinSide::West => &mut occupied_west,
        };
        let at = resolve_collision(best_at, best_side, cfg, occ);
        let coord = match best_side {
            PinSide::North | PinSide::South => at.x,
            PinSide::East | PinSide::West => at.y,
        };
        occ.push(coord);
        out.push(PinPlacement {
            name: req.name.clone(),
            side: best_side,
            at,
        });
    }
    out
}

fn centroid(pts: &[Point]) -> Point {
    let n = pts.len() as i64;
    let sx: i64 = pts.iter().map(|p| p.x).sum();
    let sy: i64 = pts.iter().map(|p| p.y).sum();
    Point::new(sx / n, sy / n)
}

/// Select the closest face of `bbox` to `centroid`, restricted to
/// the sides in `allowed`. Returns `None` if `allowed` is empty;
/// callers in this module gate on `req.allowed_sides.is_empty()`
/// before calling, so this is the only way the lookup can fail.
fn pick_side(centroid: Point, bbox: Bbox, allowed: &[PinSide]) -> Option<(PinSide, Point)> {
    // Compute distance from centroid to each allowed-side line; pick
    // the closest.
    let mut best: Option<(PinSide, Point, i64)> = None;
    for &side in allowed {
        let candidate = match side {
            PinSide::North => Point::new(
                centroid.x.clamp(bbox.min.x, bbox.max.x),
                bbox.max.y,
            ),
            PinSide::South => Point::new(
                centroid.x.clamp(bbox.min.x, bbox.max.x),
                bbox.min.y,
            ),
            PinSide::East => Point::new(
                bbox.max.x,
                centroid.y.clamp(bbox.min.y, bbox.max.y),
            ),
            PinSide::West => Point::new(
                bbox.min.x,
                centroid.y.clamp(bbox.min.y, bbox.max.y),
            ),
        };
        let d = (centroid.x - candidate.x).abs() + (centroid.y - candidate.y).abs();
        match best {
            None => best = Some((side, candidate, d)),
            Some((_, _, prev)) if d < prev => best = Some((side, candidate, d)),
            _ => {}
        }
    }
    best.map(|(s, p, _)| (s, p))
}

fn resolve_collision(
    desired: Point,
    side: PinSide,
    cfg: &PinOptConfig,
    occupied: &[i64],
) -> Point {
    let coord = match side {
        PinSide::North | PinSide::South => desired.x,
        PinSide::East | PinSide::West => desired.y,
    };
    let mut snapped = if cfg.track_pitch > 0 {
        ((coord + cfg.track_pitch / 2) / cfg.track_pitch) * cfg.track_pitch
    } else {
        coord
    };
    // Bump until no collision within pin_spacing.
    let bound_lo;
    let bound_hi;
    match side {
        PinSide::North | PinSide::South => {
            bound_lo = cfg.block_bbox.min.x;
            bound_hi = cfg.block_bbox.max.x;
        }
        PinSide::East | PinSide::West => {
            bound_lo = cfg.block_bbox.min.y;
            bound_hi = cfg.block_bbox.max.y;
        }
    };
    let step = cfg.track_pitch.max(1);
    let pin_spacing = cfg.pin_spacing.max(0);
    let mut tries = 0;
    while occupied.iter().any(|&o| (snapped - o).abs() < pin_spacing) && tries < 1000 {
        snapped += step;
        if snapped > bound_hi {
            snapped = bound_lo;
        }
        tries += 1;
    }
    match side {
        PinSide::North | PinSide::South => Point::new(snapped, desired.y),
        PinSide::East | PinSide::West => Point::new(desired.x, snapped),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> PinOptConfig {
        PinOptConfig {
            block_bbox: Bbox::new(Point::new(0, 0), Point::new(100, 100)),
            track_pitch: 5,
            pin_spacing: 10,
        }
    }

    #[test]
    fn east_centroid_picks_east_side() {
        let req = PinReq {
            name: "p".into(),
            net_pins: vec![Point::new(200, 50)],
            allowed_sides: vec![PinSide::North, PinSide::South, PinSide::East, PinSide::West],
        };
        let placed = pin_opt(&[req], &cfg());
        assert_eq!(placed[0].side, PinSide::East);
        assert_eq!(placed[0].at.x, 100);
    }

    #[test]
    fn west_centroid_picks_west_side() {
        let req = PinReq {
            name: "p".into(),
            net_pins: vec![Point::new(-50, 50)],
            allowed_sides: vec![PinSide::North, PinSide::South, PinSide::East, PinSide::West],
        };
        let placed = pin_opt(&[req], &cfg());
        assert_eq!(placed[0].side, PinSide::West);
    }

    #[test]
    fn pin_collision_resolved_via_step() {
        let reqs = vec![
            PinReq {
                name: "a".into(),
                net_pins: vec![Point::new(200, 50)],
                allowed_sides: vec![PinSide::East],
            },
            PinReq {
                name: "b".into(),
                net_pins: vec![Point::new(200, 50)],
                allowed_sides: vec![PinSide::East],
            },
        ];
        let placed = pin_opt(&reqs, &cfg());
        // Both on east side at distinct y.
        assert_eq!(placed.len(), 2);
        assert!(placed[0].at.y != placed[1].at.y);
    }

    #[test]
    fn snap_to_track_pitch() {
        let req = PinReq {
            name: "p".into(),
            net_pins: vec![Point::new(200, 53)],
            allowed_sides: vec![PinSide::East],
        };
        let placed = pin_opt(&[req], &cfg());
        assert_eq!(placed[0].at.y % 5, 0);
    }

    #[test]
    fn empty_input_yields_empty_output() {
        assert!(pin_opt(&[], &cfg()).is_empty());
    }
}
