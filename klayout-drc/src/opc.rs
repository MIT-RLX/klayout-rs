//! Optical-Proximity-Correction (OPC) primitives.
//!
//! OPC pre-distorts the design at mask-write time so the printed
//! image lands closer to the intended geometry after lithography
//! diffraction shrinks line ends and pulls in corners. The full
//! production OPC pass is iterative + model-driven (lithography
//! solver in the loop); v1 ships the *primitive* operations a
//! light-OPC pass uses:
//!
//! * [`hammerhead`] — line-end extension. Each wire endpoint gets a
//!   small T-shaped enlargement to compensate for foreshortening.
//! * [`serif`] — corner serif. Each convex corner gets a small
//!   square added to keep the corner from rounding too aggressively.
//! * [`sraf`] — sub-resolution assist features. Auxiliary parallel
//!   stripes placed at `sraf_offset` from each long straight edge,
//!   below the print threshold but improving the main feature's
//!   process window.
//!
//! Each function returns a `Region` of polygons that should be
//! *added* to the original layer for the OPC-corrected mask.

use klayout_core::{Bbox, Point, Polygon};
use klayout_geom::{merge, Region};

#[derive(Clone, Debug)]
pub struct OpcConfig {
    /// Hammerhead extension amount (DBU) — how far the T-cap reaches
    /// past the original endpoint.
    pub hammerhead_extension: i64,
    /// Hammerhead width — perpendicular size of the T-cap.
    pub hammerhead_width: i64,
    /// Corner-serif size (square dimension at each convex corner).
    pub serif_size: i64,
    /// Distance from the main feature edge at which an SRAF is placed.
    pub sraf_offset: i64,
    /// SRAF width.
    pub sraf_width: i64,
    /// Minimum length of a feature edge that gets an SRAF.
    pub sraf_min_edge_length: i64,
}

impl Default for OpcConfig {
    fn default() -> Self {
        Self {
            hammerhead_extension: 5,
            hammerhead_width: 8,
            serif_size: 3,
            sraf_offset: 10,
            sraf_width: 2,
            sraf_min_edge_length: 50,
        }
    }
}

/// Add hammerheads at every wire endpoint. v1 detects endpoints as
/// degree-1 vertices on the *path* output of [`klayout_geom::path`]
/// — for region inputs we use the polygon hull's "narrow ends" (a
/// pair of parallel edges close to a perpendicular short edge).
pub fn hammerhead(r: &Region, cfg: &OpcConfig) -> Region {
    if r.is_empty() || cfg.hammerhead_extension <= 0 {
        return Region::empty();
    }
    let merged = merge(r);
    let mut adds: Vec<Polygon> = Vec::new();
    for p in merged.polygons() {
        let n = p.hull.len();
        if n < 4 {
            continue;
        }
        for i in 0..n {
            let a = p.hull[i];
            let b = p.hull[(i + 1) % n];
            // Look for short edges that are tip ends — heuristic:
            // edge length ≤ hammerhead_width × 1.5.
            let len = (b.x - a.x).abs() + (b.y - a.y).abs();
            if len > cfg.hammerhead_width * 3 / 2 {
                continue;
            }
            let mid = Point::new((a.x + b.x) / 2, (a.y + b.y) / 2);
            // Direction perpendicular to the short edge — outward.
            let prev = p.hull[(i + n - 1) % n];
            let dx = mid.x - prev.x;
            let dy = mid.y - prev.y;
            let outward = match (dx.signum(), dy.signum()) {
                (s, 0) => (s, 0),
                (0, s) => (0, s),
                _ => continue,
            };
            let ext = cfg.hammerhead_extension;
            let half = cfg.hammerhead_width / 2;
            let cap_bbox = if outward.0 != 0 {
                // Edge is vertical, extend in x.
                Bbox::new(
                    Point::new(mid.x, mid.y - half),
                    Point::new(mid.x + outward.0 * ext, mid.y + half),
                )
            } else {
                Bbox::new(
                    Point::new(mid.x - half, mid.y),
                    Point::new(mid.x + half, mid.y + outward.1 * ext),
                )
            };
            adds.push(Polygon::rect(canonicalize_bbox(cap_bbox)));
        }
    }
    if adds.is_empty() {
        Region::empty()
    } else {
        merge(&Region::from_polygons(adds))
    }
}

/// Add serifs at every convex corner.
pub fn serif(r: &Region, cfg: &OpcConfig) -> Region {
    if r.is_empty() || cfg.serif_size <= 0 {
        return Region::empty();
    }
    let merged = merge(r);
    let mut adds: Vec<Polygon> = Vec::new();
    let s = cfg.serif_size;
    for p in merged.polygons() {
        let n = p.hull.len();
        if n < 3 {
            continue;
        }
        for i in 0..n {
            let prev = p.hull[(i + n - 1) % n];
            let cur = p.hull[i];
            let next = p.hull[(i + 1) % n];
            // Convex if the turn is right (CW polygon → cross < 0).
            let cross = (cur.x - prev.x) as i128 * (next.y - cur.y) as i128
                - (cur.y - prev.y) as i128 * (next.x - cur.x) as i128;
            if cross >= 0 {
                continue;
            }
            // For CW polygon, outside-direction is sum of perp-left of
            // incoming + perp-left of outgoing (the exteriors of each
            // edge).
            let in_dx = (cur.x - prev.x).signum();
            let in_dy = (cur.y - prev.y).signum();
            let out_dx = (next.x - cur.x).signum();
            let out_dy = (next.y - cur.y).signum();
            // perp-left of (dx, dy) for CW interior-on-right is (-dy, dx).
            let ext_x = (-in_dy) + (-out_dy);
            let ext_y = in_dx + out_dx;
            let bias_x = ext_x.signum();
            let bias_y = ext_y.signum();
            if bias_x == 0 && bias_y == 0 {
                continue;
            }
            let cap = Bbox::new(
                Point::new(cur.x.min(cur.x + bias_x * s), cur.y.min(cur.y + bias_y * s)),
                Point::new(cur.x.max(cur.x + bias_x * s), cur.y.max(cur.y + bias_y * s)),
            );
            let cb = canonicalize_bbox(cap);
            if cb.width() > 0 && cb.height() > 0 {
                adds.push(Polygon::rect(cb));
            }
        }
    }
    if adds.is_empty() {
        Region::empty()
    } else {
        merge(&Region::from_polygons(adds))
    }
}

/// Add SRAFs (parallel auxiliary stripes) along every long
/// axis-aligned edge.
pub fn sraf(r: &Region, cfg: &OpcConfig) -> Region {
    if r.is_empty() || cfg.sraf_width <= 0 {
        return Region::empty();
    }
    let merged = merge(r);
    let mut adds: Vec<Polygon> = Vec::new();
    for p in merged.polygons() {
        let n = p.hull.len();
        for i in 0..n {
            let a = p.hull[i];
            let b = p.hull[(i + 1) % n];
            let len = (b.x - a.x).abs() + (b.y - a.y).abs();
            if len < cfg.sraf_min_edge_length {
                continue;
            }
            // Only axis-aligned edges in v1.
            if a.y == b.y {
                // Horizontal edge — SRAF parallel to it.
                let prev = p.hull[(i + n - 1) % n];
                let outward_y = (a.y - prev.y).signum();
                if outward_y == 0 {
                    continue;
                }
                let y0 = a.y + outward_y * cfg.sraf_offset;
                let y1 = y0 + outward_y * cfg.sraf_width;
                adds.push(Polygon::rect(canonicalize_bbox(Bbox::new(
                    Point::new(a.x.min(b.x), y0.min(y1)),
                    Point::new(a.x.max(b.x), y0.max(y1)),
                ))));
            } else if a.x == b.x {
                let prev = p.hull[(i + n - 1) % n];
                let outward_x = (a.x - prev.x).signum();
                if outward_x == 0 {
                    continue;
                }
                let x0 = a.x + outward_x * cfg.sraf_offset;
                let x1 = x0 + outward_x * cfg.sraf_width;
                adds.push(Polygon::rect(canonicalize_bbox(Bbox::new(
                    Point::new(x0.min(x1), a.y.min(b.y)),
                    Point::new(x0.max(x1), a.y.max(b.y)),
                ))));
            }
        }
    }
    if adds.is_empty() {
        Region::empty()
    } else {
        merge(&Region::from_polygons(adds))
    }
}

fn canonicalize_bbox(b: Bbox) -> Bbox {
    Bbox::new(
        Point::new(b.min.x.min(b.max.x), b.min.y.min(b.max.y)),
        Point::new(b.min.x.max(b.max.x), b.min.y.max(b.max.y)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Polygon {
        Polygon::rect(Bbox::new(Point::new(x0, y0), Point::new(x1, y1)))
    }

    #[test]
    fn empty_region_yields_nothing() {
        let cfg = OpcConfig::default();
        assert!(hammerhead(&Region::empty(), &cfg).is_empty());
        assert!(serif(&Region::empty(), &cfg).is_empty());
        assert!(sraf(&Region::empty(), &cfg).is_empty());
    }

    #[test]
    fn serif_adds_caps_at_corners() {
        let r = Region::from_polygons([rect(0, 0, 100, 100)]);
        let s = serif(&r, &OpcConfig::default());
        // Square has 4 convex corners → expect ≥ 4 serif additions
        // (some may merge if overlapping).
        assert!(!s.is_empty());
    }

    #[test]
    fn sraf_avoids_short_edges() {
        let cfg = OpcConfig {
            sraf_min_edge_length: 1000,
            ..OpcConfig::default()
        };
        let r = Region::from_polygons([rect(0, 0, 50, 50)]);
        // No edge meets the 1000-DBU minimum.
        assert!(sraf(&r, &cfg).is_empty());
    }

    #[test]
    fn sraf_emits_parallel_stripes() {
        let r = Region::from_polygons([rect(0, 0, 1000, 50)]);
        let s = sraf(&r, &OpcConfig::default());
        // Long horizontal edges → SRAFs.
        assert!(!s.is_empty());
    }
}
