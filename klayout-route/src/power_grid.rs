//! Power grid (VDD/VSS) generation.
//!
//! Every digital chip needs a power-distribution network (PDN): a
//! regular pattern of metal stripes (typically alternating VDD / VSS)
//! across upper layers, with via stacks down to the standard-cell
//! M1 rails. This module emits the geometry — pitch, width, ring
//! offset, layer choice — as `RouteSegment`s the rest of the pipeline
//! consumes (stylize → write_def_full → DEF SPECIALNETS).
//!
//! v1 emits:
//! * **Vertical stripes** on `vertical_layer` at `vertical_pitch`,
//!   alternating VDD / VSS.
//! * **Horizontal stripes** on `horizontal_layer` at
//!   `horizontal_pitch`, alternating VDD / VSS.
//! * **Via stack** wherever a VDD vertical crosses a VDD horizontal
//!   (and same for VSS) — emits a `Via` per layer-pair traversed.
//! * **Optional core ring** — a single VDD + VSS ring around `core_bbox`
//!   on `vertical_layer` (with offset / width).
//!
//! Out of scope for v1: tap insertion, decap insertion, IR-drop-driven
//! pitch tuning, hierarchical PDN over partitioned blocks. Each is a
//! straightforward extension.

use crate::multilayer::RouteSegment;
use klayout_core::{Bbox, Point};
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct PowerGridConfig {
    /// Layer index for vertical stripes.
    pub vertical_layer: usize,
    /// Pitch (DBU) between consecutive vertical stripes.
    pub vertical_pitch: i64,
    /// Width (DBU) of each vertical stripe.
    pub vertical_width: i64,
    /// Layer index for horizontal stripes.
    pub horizontal_layer: usize,
    pub horizontal_pitch: i64,
    pub horizontal_width: i64,
    /// Via cut count to use at every PG intersection.
    pub via_cut_count: u32,
    /// Optional ring on `vertical_layer` around `core_bbox`. When
    /// `Some`, two rings are emitted (VDD outer, VSS inner) at
    /// `ring_offset` from the core boundary, `ring_width` wide.
    pub ring: Option<RingConfig>,
}

#[derive(Clone, Debug)]
pub struct RingConfig {
    pub ring_offset: i64,
    pub ring_width: i64,
}

#[derive(Clone, Debug)]
pub struct PowerGrid {
    pub vdd_segments: Vec<RouteSegment>,
    pub vss_segments: Vec<RouteSegment>,
    pub vdd_name: SmolStr,
    pub vss_name: SmolStr,
}

impl PowerGrid {
    pub fn all_segments(&self) -> impl Iterator<Item = &RouteSegment> {
        self.vdd_segments.iter().chain(self.vss_segments.iter())
    }

    pub fn segment_count(&self) -> usize {
        self.vdd_segments.len() + self.vss_segments.len()
    }
}

/// Generate a power grid over the area `core_bbox`.
pub fn generate_power_grid(core_bbox: Bbox, cfg: &PowerGridConfig) -> PowerGrid {
    let mut vdd_segments: Vec<RouteSegment> = Vec::new();
    let mut vss_segments: Vec<RouteSegment> = Vec::new();

    // Vertical stripes: alternate VDD/VSS across the layout.
    let v_pitch = cfg.vertical_pitch.max(1);
    let mut x = core_bbox.min.x;
    let mut idx_v = 0;
    let mut vdd_xs: Vec<i64> = Vec::new();
    let mut vss_xs: Vec<i64> = Vec::new();
    while x <= core_bbox.max.x {
        let is_vdd = idx_v % 2 == 0;
        let stripe = RouteSegment::Wire {
            layer_idx: cfg.vertical_layer,
            points: vec![Point::new(x, core_bbox.min.y), Point::new(x, core_bbox.max.y)],
        };
        if is_vdd {
            vdd_segments.push(stripe);
            vdd_xs.push(x);
        } else {
            vss_segments.push(stripe);
            vss_xs.push(x);
        }
        x += v_pitch;
        idx_v += 1;
    }

    // Horizontal stripes: alternate VDD/VSS.
    let h_pitch = cfg.horizontal_pitch.max(1);
    let mut y = core_bbox.min.y;
    let mut idx_h = 0;
    let mut vdd_ys: Vec<i64> = Vec::new();
    let mut vss_ys: Vec<i64> = Vec::new();
    while y <= core_bbox.max.y {
        let is_vdd = idx_h % 2 == 0;
        let stripe = RouteSegment::Wire {
            layer_idx: cfg.horizontal_layer,
            points: vec![Point::new(core_bbox.min.x, y), Point::new(core_bbox.max.x, y)],
        };
        if is_vdd {
            vdd_segments.push(stripe);
            vdd_ys.push(y);
        } else {
            vss_segments.push(stripe);
            vss_ys.push(y);
        }
        y += h_pitch;
        idx_h += 1;
    }

    // Via stack at every same-net intersection.
    for &xv in &vdd_xs {
        for &yh in &vdd_ys {
            vdd_segments.push(RouteSegment::Via {
                at: Point::new(xv, yh),
                from_layer: cfg.horizontal_layer.min(cfg.vertical_layer),
                to_layer: cfg.horizontal_layer.max(cfg.vertical_layer),
                cut_count: cfg.via_cut_count,
            });
        }
    }
    for &xv in &vss_xs {
        for &yh in &vss_ys {
            vss_segments.push(RouteSegment::Via {
                at: Point::new(xv, yh),
                from_layer: cfg.horizontal_layer.min(cfg.vertical_layer),
                to_layer: cfg.horizontal_layer.max(cfg.vertical_layer),
                cut_count: cfg.via_cut_count,
            });
        }
    }

    // Optional core ring (VDD outer, VSS inner) on vertical_layer.
    if let Some(ring) = &cfg.ring {
        let outer = expand_bbox(core_bbox, ring.ring_offset);
        let inner = expand_bbox(core_bbox, ring.ring_offset - ring.ring_width.max(1));
        vdd_segments.extend(rect_ring_as_wires(outer, cfg.vertical_layer));
        vss_segments.extend(rect_ring_as_wires(inner, cfg.vertical_layer));
    }

    PowerGrid {
        vdd_segments,
        vss_segments,
        vdd_name: SmolStr::from("VDD"),
        vss_name: SmolStr::from("VSS"),
    }
}

fn expand_bbox(b: Bbox, by: i64) -> Bbox {
    Bbox::new(
        Point::new(b.min.x - by, b.min.y - by),
        Point::new(b.max.x + by, b.max.y + by),
    )
}

fn rect_ring_as_wires(b: Bbox, layer_idx: usize) -> Vec<RouteSegment> {
    vec![
        // bottom
        RouteSegment::Wire {
            layer_idx,
            points: vec![Point::new(b.min.x, b.min.y), Point::new(b.max.x, b.min.y)],
        },
        // right
        RouteSegment::Wire {
            layer_idx,
            points: vec![Point::new(b.max.x, b.min.y), Point::new(b.max.x, b.max.y)],
        },
        // top
        RouteSegment::Wire {
            layer_idx,
            points: vec![Point::new(b.max.x, b.max.y), Point::new(b.min.x, b.max.y)],
        },
        // left
        RouteSegment::Wire {
            layer_idx,
            points: vec![Point::new(b.min.x, b.max.y), Point::new(b.min.x, b.min.y)],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> PowerGridConfig {
        PowerGridConfig {
            vertical_layer: 1,
            vertical_pitch: 100,
            vertical_width: 4,
            horizontal_layer: 0,
            horizontal_pitch: 50,
            horizontal_width: 2,
            via_cut_count: 4,
            ring: None,
        }
    }

    #[test]
    fn alternates_vdd_vss_stripes() {
        let pg = generate_power_grid(
            Bbox::new(Point::new(0, 0), Point::new(400, 200)),
            &cfg(),
        );
        // Verticals at x in {0, 100, 200, 300, 400} → 5 stripes,
        // alternating VDD/VSS starting VDD: 3 VDD, 2 VSS.
        let v_vdd = pg.vdd_segments.iter().filter(|s| matches!(s, RouteSegment::Wire { layer_idx: 1, .. })).count();
        let v_vss = pg.vss_segments.iter().filter(|s| matches!(s, RouteSegment::Wire { layer_idx: 1, .. })).count();
        assert_eq!(v_vdd, 3);
        assert_eq!(v_vss, 2);
    }

    #[test]
    fn vias_at_same_net_intersections_only() {
        let pg = generate_power_grid(
            Bbox::new(Point::new(0, 0), Point::new(400, 200)),
            &cfg(),
        );
        // 3 VDD verticals × 3 VDD horizontals (y=0,100,200) = 9 vias.
        let vdd_via = pg
            .vdd_segments
            .iter()
            .filter(|s| matches!(s, RouteSegment::Via { .. }))
            .count();
        assert_eq!(vdd_via, 9);
        // 2 VSS verticals × 2 VSS horizontals (y=50,150) = 4 vias.
        let vss_via = pg
            .vss_segments
            .iter()
            .filter(|s| matches!(s, RouteSegment::Via { .. }))
            .count();
        assert_eq!(vss_via, 4);
    }

    #[test]
    fn ring_adds_wire_loops_when_configured() {
        let mut c = cfg();
        c.ring = Some(RingConfig {
            ring_offset: 20,
            ring_width: 10,
        });
        let pg = generate_power_grid(
            Bbox::new(Point::new(0, 0), Point::new(100, 100)),
            &c,
        );
        // 1 VDD vertical (x=0; x=100 is VSS) + 4 ring wires = 5.
        let ring_wires_vdd = pg
            .vdd_segments
            .iter()
            .filter(|s| matches!(s, RouteSegment::Wire { layer_idx: 1, .. }))
            .count();
        assert_eq!(ring_wires_vdd, 5);
    }

    #[test]
    fn empty_bbox_yields_no_segments() {
        let pg = generate_power_grid(Bbox::EMPTY, &cfg());
        assert_eq!(pg.segment_count(), 0);
    }
}
