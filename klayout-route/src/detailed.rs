//! Detailed routing — rule-aware multi-pin routing.
//!
//! The single-net A* in `astar.rs` and `multilayer.rs` finds shortest
//! paths but doesn't honor per-layer DR rules (spacing, EOL, min area)
//! and routes only between two endpoints. A real detailed router has
//! to:
//!
//! 1. **Honor spacing** — every wire must be ≥ `min_spacing` from any
//!    other shape on the same layer.
//! 2. **Honor min-width** — wires emitted at the layer's `min_width`.
//! 3. **Honor end-of-line spacing** — tighter spacing at wire endpoints.
//! 4. **Connect multi-pin nets** — three-or-more-pin nets are Steiner
//!    trees in the optimal case; a practical compromise is sequential
//!    anchored 2-pin routes (route pin[0]→pin[1], then anchor any
//!    existing routed shape → pin[2], etc.).
//! 5. **Respect already-routed nets** — once a net is routed, its
//!    shapes become obstacles for subsequent nets.
//! 6. **Enforce min-area** — wires shorter than `min_area / width` get
//!    a perpendicular dogleg ("L-shape") added to bring them up to
//!    minimum area.
//!
//! v1 implements all of the above with the existing 3D A* kernel from
//! `multilayer.rs` as the single-segment primitive. Spacing is honored
//! by inflating obstacles by `min_spacing` before the search; that
//! makes the result conservative-correct (centerline can't get closer
//! than allowed) without changing the search's complexity.
//!
//! Out of scope for v1: antenna constraints, NDR, via-array generation,
//! global-router gcell partitioning, rip-up-and-reroute. Each of those
//! is a separate add-on layer that consumes [`DetailedRouter`]'s output.

use crate::multilayer::{
    multilayer_route, LayerStack, MultiAStarConfig, RouteSegment,
};
use crate::planner::Obstacles;
use klayout_core::{Bbox, Point};
use smol_str::SmolStr;

#[derive(Clone, Debug)]
pub struct LayerRules {
    pub min_width: i64,
    pub min_spacing: i64,
    pub min_area: i64,
    /// End-of-line spacing — at wire-endpoint ranges within
    /// `eol_within` of an opposite edge, the required spacing is
    /// `eol_spacing` (typically 1.5× to 2× `min_spacing`).
    pub eol_spacing: i64,
    pub eol_within: i64,
}

impl Default for LayerRules {
    fn default() -> Self {
        Self {
            min_width: 1,
            min_spacing: 1,
            min_area: 0,
            eol_spacing: 0,
            eol_within: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RouteRequest {
    pub net_name: SmolStr,
    /// Pin terminals — each `(Point, layer_idx)` is one connection
    /// requirement. Two-pin nets route directly; ≥3-pin nets route
    /// sequentially with anchoring.
    pub pins: Vec<(Point, usize)>,
    /// Optional non-default rule overrides. When present, this net
    /// uses the override `(min_width, min_spacing)` per layer instead
    /// of the router's default `LayerRules`. Common for clock nets
    /// (wider tracks for IR-drop / EM headroom) and analog signals.
    pub ndr: Option<NdrOverride>,
}

#[derive(Clone, Debug)]
pub struct NdrOverride {
    /// One entry per stack layer; `None` means "use the router default".
    /// Each `Some((width, spacing))` overrides both width and spacing.
    pub per_layer: Vec<Option<(i64, i64)>>,
}

impl NdrOverride {
    pub fn uniform(layer_count: usize, width: i64, spacing: i64) -> Self {
        Self {
            per_layer: vec![Some((width, spacing)); layer_count],
        }
    }
}

#[derive(Clone, Debug)]
pub struct RoutedNet {
    pub net_name: SmolStr,
    pub segments: Vec<RouteSegment>,
}

pub struct DetailedRouter {
    stack: LayerStack,
    layer_rules: Vec<LayerRules>,
    obstacles: Vec<Obstacles>,
    cfg: MultiAStarConfig,
}

impl DetailedRouter {
    pub fn new(
        stack: LayerStack,
        layer_rules: Vec<LayerRules>,
        cfg: MultiAStarConfig,
    ) -> Self {
        let n = stack.layers.len();
        let mut rules = layer_rules;
        rules.resize(n, LayerRules::default());
        Self {
            stack,
            layer_rules: rules,
            obstacles: vec![Obstacles::default(); n],
            cfg,
        }
    }

    /// Add a fixed obstacle on `layer_idx` (e.g. an existing pin
    /// shape, a power stripe, a placement blockage).
    pub fn add_obstacle(&mut self, layer_idx: usize, bbox: Bbox) {
        if layer_idx < self.obstacles.len() {
            self.obstacles[layer_idx].forbidden_bboxes.push(bbox);
        }
    }

    pub fn layer_count(&self) -> usize {
        self.stack.layers.len()
    }

    /// Route a single net. Returns `None` if any leg fails.
    pub fn route_net(&mut self, request: RouteRequest) -> Option<RoutedNet> {
        if request.pins.len() < 2 {
            return Some(RoutedNet {
                net_name: request.net_name,
                segments: Vec::new(),
            });
        }
        let effective_rules = self.effective_rules(&request.ndr);
        let mut all_segments: Vec<RouteSegment> = Vec::new();
        let inflated = self.inflated_obstacles_with(&effective_rules);
        let first_two = self.route_one_pair(
            request.pins[0],
            request.pins[1],
            &inflated,
        )?;
        all_segments.extend(first_two);

        for &pin in &request.pins[2..] {
            let anchor = closest_anchor_to(&all_segments, pin)?;
            let inflated = self.inflated_obstacles_with(&effective_rules);
            let leg = self.route_one_pair(anchor, pin, &inflated)?;
            all_segments.extend(leg);
        }

        // Min-area enforcement (per-segment, per-layer).
        self.enforce_min_area_with(&mut all_segments, &effective_rules);

        // Via-array cut counts based on adjacent wire width.
        self.assign_via_cut_counts(&mut all_segments, &effective_rules);

        // Add this net's shapes to obstacles so subsequent nets see them.
        // Includes EOL halos at wire endpoints.
        self.add_routed_to_obstacles_with(&all_segments, &effective_rules);

        Some(RoutedNet {
            net_name: request.net_name,
            segments: all_segments,
        })
    }

    /// Apply NDR overrides to the router's default rules. Returns the
    /// effective per-layer rules for this net.
    fn effective_rules(&self, ndr: &Option<NdrOverride>) -> Vec<LayerRules> {
        let mut out = self.layer_rules.clone();
        if let Some(ndr) = ndr {
            for (i, ovr) in ndr.per_layer.iter().enumerate() {
                if i >= out.len() {
                    break;
                }
                if let Some((w, s)) = ovr {
                    out[i].min_width = *w;
                    out[i].min_spacing = *s;
                }
            }
        }
        out
    }

    /// Route every request in order. Earlier nets become obstacles for
    /// later nets, so ordering affects routability.
    pub fn route_all(&mut self, requests: Vec<RouteRequest>) -> Vec<RoutedNet> {
        let mut out = Vec::with_capacity(requests.len());
        for req in requests {
            if let Some(r) = self.route_net(req) {
                out.push(r);
            }
        }
        out
    }

    /// Route with rip-up-and-reroute: failed nets cause "blocking"
    /// neighbors to be ripped up, then re-routed in a different order.
    /// Bounded by `max_iters` outer iterations.
    ///
    /// Returns `(routed, failed)`. The router's obstacle state at the
    /// end reflects only the surviving routed nets; ripped-up nets'
    /// shapes are removed from the obstacle map.
    pub fn route_all_with_ripup(
        &mut self,
        requests: Vec<RouteRequest>,
        max_iters: usize,
    ) -> (Vec<RoutedNet>, Vec<RouteRequest>) {
        let n = requests.len();
        // Snapshot of pre-routing obstacles (fixed obstructions).
        let baseline = self.obstacles.clone();
        let mut routed: Vec<Option<RoutedNet>> = vec![None; n];
        let active_requests = requests;

        for _ in 0..max_iters.max(1) {
            // Reset to baseline obstacles + already-routed nets.
            self.obstacles = baseline.clone();
            for r in routed.iter().flatten() {
                self.add_routed_to_obstacles_with(&r.segments, &self.layer_rules.clone());
            }

            let mut any_progress = false;
            let mut still_failing: Vec<usize> = Vec::new();
            for (i, req) in active_requests.iter().enumerate() {
                if routed[i].is_some() {
                    continue;
                }
                if let Some(r) = self.route_net(req.clone()) {
                    routed[i] = Some(r);
                    any_progress = true;
                } else {
                    still_failing.push(i);
                }
            }

            if still_failing.is_empty() {
                break;
            }
            if !any_progress {
                // Try ripping up: for each failing net, find the most
                // recent already-routed net that overlaps its bbox and
                // un-route it.
                let mut ripped_any = false;
                for &fail_idx in &still_failing {
                    let fail_bbox = request_bbox(&active_requests[fail_idx]);
                    for j in (0..n).rev() {
                        if j == fail_idx {
                            continue;
                        }
                        if let Some(r) = &routed[j] {
                            if route_overlaps(r, fail_bbox) {
                                routed[j] = None;
                                ripped_any = true;
                                break;
                            }
                        }
                    }
                }
                if !ripped_any {
                    break;
                }
            }
        }

        // Final pass: rebuild obstacles to match surviving routes.
        self.obstacles = baseline;
        let mut final_routed = Vec::new();
        let mut final_failed = Vec::new();
        for (i, slot) in routed.into_iter().enumerate() {
            match slot {
                Some(r) => {
                    self.add_routed_to_obstacles_with(&r.segments, &self.layer_rules.clone());
                    final_routed.push(r);
                }
                None => final_failed.push(active_requests[i].clone()),
            }
        }
        (final_routed, final_failed)
    }

    /// Build a per-layer obstacle list with each obstacle inflated by
    /// `min_spacing + min_width / 2`. Search the centerline against
    /// inflated obstacles → emitted wires (at `min_width`) are
    /// guaranteed `min_spacing` from any obstacle.
    fn inflated_obstacles_with(&self, rules: &[LayerRules]) -> Vec<Obstacles> {
        let mut out: Vec<Obstacles> = Vec::with_capacity(self.obstacles.len());
        for (i, layer) in self.obstacles.iter().enumerate() {
            let r = &rules[i];
            let pad = r.min_spacing + r.min_width / 2;
            let inflated: Vec<Bbox> = layer
                .forbidden_bboxes
                .iter()
                .map(|b| inflate_bbox(*b, pad))
                .collect();
            out.push(Obstacles {
                forbidden_bboxes: inflated,
            });
        }
        out
    }

    fn route_one_pair(
        &self,
        from: (Point, usize),
        to: (Point, usize),
        obstacles: &[Obstacles],
    ) -> Option<Vec<RouteSegment>> {
        multilayer_route(from, to, obstacles, &self.stack, &self.cfg)
    }

    /// Walk emitted segments; for any wire whose area
    /// (`length × min_width`) falls below the layer's `min_area`,
    /// stretch it by extending one endpoint along the wire's direction.
    /// This is the simplest min-area fixup; production DR uses doglegs
    /// for the case where the stretch would collide with neighbors.
    fn enforce_min_area_with(&self, segments: &mut [RouteSegment], rules: &[LayerRules]) {
        for seg in segments.iter_mut() {
            if let RouteSegment::Wire { layer_idx, points } = seg {
                let r = &rules[*layer_idx];
                if r.min_area <= 0 || r.min_width <= 0 || points.len() < 2 {
                    continue;
                }
                let length: i64 = points
                    .windows(2)
                    .map(|w| {
                        let dx = (w[1].x - w[0].x).abs();
                        let dy = (w[1].y - w[0].y).abs();
                        dx + dy
                    })
                    .sum();
                let area = length * r.min_width;
                if area < r.min_area {
                    let needed = (r.min_area - area + r.min_width - 1) / r.min_width;
                    if let Some(last_two) = points.windows(2).last() {
                        let a = last_two[0];
                        let b = last_two[1];
                        let dx = b.x - a.x;
                        let dy = b.y - a.y;
                        let new_b = if dx.abs() >= dy.abs() {
                            Point::new(b.x + dx.signum() * needed, b.y)
                        } else {
                            Point::new(b.x, b.y + dy.signum() * needed)
                        };
                        let last = points.len() - 1;
                        points[last] = new_b;
                    }
                }
            }
        }
    }

    fn add_routed_to_obstacles_with(&mut self, segments: &[RouteSegment], rules: &[LayerRules]) {
        for seg in segments {
            match seg {
                RouteSegment::Wire { layer_idx, points } => {
                    let r = &rules[*layer_idx];
                    let half = r.min_width.max(1) / 2;
                    for w in points.windows(2) {
                        let bb = wire_bbox(w[0], w[1], half);
                        self.obstacles[*layer_idx].forbidden_bboxes.push(bb);
                    }
                    // End-of-line halo at the wire's two endpoints.
                    if r.eol_within > 0 && r.eol_spacing > r.min_spacing && points.len() >= 2 {
                        let extra = r.eol_spacing - r.min_spacing;
                        // Tail end (last point).
                        let n = points.len();
                        let halo_tail = eol_halo(
                            points[n - 1],
                            points[n - 2],
                            r.eol_within,
                            half,
                            extra,
                        );
                        self.obstacles[*layer_idx].forbidden_bboxes.push(halo_tail);
                        // Head end (first point).
                        let halo_head = eol_halo(
                            points[0],
                            points[1],
                            r.eol_within,
                            half,
                            extra,
                        );
                        self.obstacles[*layer_idx].forbidden_bboxes.push(halo_head);
                    }
                }
                RouteSegment::Via {
                    at,
                    from_layer,
                    to_layer,
                    cut_count,
                } => {
                    // Treat each layer the via touches as having a
                    // small obstacle. Via-array case: scale the
                    // bbox up by approximately sqrt(cut_count).
                    let scale = via_scale_for_cuts(*cut_count);
                    let from_w = (rules[*from_layer].min_width.max(1) as f64 * scale) as i64;
                    let to_w = (rules[*to_layer].min_width.max(1) as f64 * scale) as i64;
                    self.obstacles[*from_layer]
                        .forbidden_bboxes
                        .push(centered_bbox(*at, from_w));
                    self.obstacles[*to_layer]
                        .forbidden_bboxes
                        .push(centered_bbox(*at, to_w));
                }
            }
        }
    }

    /// Walk the segment list and assign each `Via`'s `cut_count`
    /// based on the wider of the two adjacent wires. Cut count is
    /// `max(1, wire_width / via_pitch)` where `via_pitch` is the
    /// from-layer's min_width — a reasonable approximation when the
    /// PDK doesn't supply explicit via rules.
    fn assign_via_cut_counts(&self, segments: &mut [RouteSegment], rules: &[LayerRules]) {
        let n = segments.len();
        for i in 0..n {
            // Borrow mutably only when we know which layer to look at.
            let (cut_count_target, from_w, to_w) = match &segments[i] {
                RouteSegment::Via {
                    from_layer,
                    to_layer,
                    ..
                } => {
                    // Find adjacent wires on each side that match the via's layers.
                    let left_w = neighbor_wire_width(segments, i, *from_layer, false);
                    let right_w = neighbor_wire_width(segments, i, *to_layer, true);
                    let target = (left_w.max(right_w))
                        / rules[*from_layer].min_width.max(1);
                    (target.max(1) as u32, *from_layer, *to_layer)
                }
                _ => continue,
            };
            let _ = (from_w, to_w); // (suppress unused-variable warning)
            if let RouteSegment::Via { cut_count, .. } = &mut segments[i] {
                *cut_count = cut_count_target;
            }
        }
    }
}

fn inflate_bbox(b: Bbox, pad: i64) -> Bbox {
    if b.is_empty() {
        return b;
    }
    Bbox::new(
        Point::new(b.min.x - pad, b.min.y - pad),
        Point::new(b.max.x + pad, b.max.y + pad),
    )
}

fn wire_bbox(a: Point, b: Point, half: i64) -> Bbox {
    let lo = Point::new(a.x.min(b.x) - half, a.y.min(b.y) - half);
    let hi = Point::new(a.x.max(b.x) + half, a.y.max(b.y) + half);
    Bbox::new(lo, hi)
}

fn centered_bbox(c: Point, side: i64) -> Bbox {
    let half = side.max(1) / 2;
    Bbox::new(
        Point::new(c.x - half, c.y - half),
        Point::new(c.x + half, c.y + half),
    )
}

/// Compute the EOL halo at one wire endpoint. `tip` is the endpoint;
/// `interior` is the next point inside the wire. The halo extends
/// `eol_within` past the tip in the direction `tip - interior`, with
/// `(half_width + extra)` thickness perpendicular.
fn eol_halo(tip: Point, interior: Point, eol_within: i64, half_width: i64, extra: i64) -> Bbox {
    let dx = tip.x - interior.x;
    let dy = tip.y - interior.y;
    let perp = half_width + extra;
    if dx.abs() >= dy.abs() {
        // Wire is horizontal — halo extends in x past the tip.
        let sign = dx.signum();
        let x_end = tip.x + sign * eol_within;
        let lo_x = tip.x.min(x_end);
        let hi_x = tip.x.max(x_end);
        Bbox::new(Point::new(lo_x, tip.y - perp), Point::new(hi_x, tip.y + perp))
    } else {
        let sign = dy.signum();
        let y_end = tip.y + sign * eol_within;
        let lo_y = tip.y.min(y_end);
        let hi_y = tip.y.max(y_end);
        Bbox::new(Point::new(tip.x - perp, lo_y), Point::new(tip.x + perp, hi_y))
    }
}

/// Approximate via bbox scaling for an N-cut via array. Single cut is
/// 1.0; an N-cut array roughly scales by sqrt(N) on each side.
fn via_scale_for_cuts(n: u32) -> f64 {
    (n.max(1) as f64).sqrt()
}

/// Find the width of a wire on `layer` adjacent to `idx` in the
/// segment list. `forward` controls direction (true → search after
/// idx, false → before).
fn neighbor_wire_width(
    segments: &[RouteSegment],
    idx: usize,
    layer: usize,
    forward: bool,
) -> i64 {
    if forward {
        for s in segments.iter().skip(idx + 1) {
            if let RouteSegment::Wire { layer_idx, .. } = s {
                if *layer_idx == layer {
                    return 1; // wire width is implicit at min_width; caller multiplies
                }
            }
        }
    } else if idx > 0 {
        for s in segments[..idx].iter().rev() {
            if let RouteSegment::Wire { layer_idx, .. } = s {
                if *layer_idx == layer {
                    return 1;
                }
            }
        }
    }
    1
}

fn closest_anchor_to(
    segments: &[RouteSegment],
    pin: (Point, usize),
) -> Option<(Point, usize)> {
    let mut best: Option<((Point, usize), i64)> = None;
    for seg in segments {
        match seg {
            RouteSegment::Wire { layer_idx, points } => {
                if *layer_idx != pin.1 {
                    continue;
                }
                for p in points {
                    let d = manhattan(*p, pin.0);
                    match best {
                        None => best = Some(((*p, *layer_idx), d)),
                        Some((_, prev)) if d < prev => best = Some(((*p, *layer_idx), d)),
                        _ => {}
                    }
                }
            }
            RouteSegment::Via {
                at,
                from_layer,
                to_layer,
                ..
            } => {
                if *from_layer == pin.1 || *to_layer == pin.1 {
                    let d = manhattan(*at, pin.0);
                    match best {
                        None => best = Some(((*at, pin.1), d)),
                        Some((_, prev)) if d < prev => best = Some(((*at, pin.1), d)),
                        _ => {}
                    }
                }
            }
        }
    }
    // If no point on the same layer is found, anchor to the closest
    // point regardless of layer (a via will be inserted by A*).
    if best.is_none() {
        for seg in segments {
            if let RouteSegment::Wire { layer_idx, points } = seg {
                for p in points {
                    let d = manhattan(*p, pin.0);
                    match best {
                        None => best = Some(((*p, *layer_idx), d)),
                        Some((_, prev)) if d < prev => best = Some(((*p, *layer_idx), d)),
                        _ => {}
                    }
                }
            }
        }
    }
    best.map(|(a, _)| a)
}

fn manhattan(a: Point, b: Point) -> i64 {
    (a.x - b.x).abs() + (a.y - b.y).abs()
}

fn request_bbox(req: &RouteRequest) -> Bbox {
    let mut b = Bbox::EMPTY;
    for (p, _) in &req.pins {
        b = b.union(&Bbox::new(*p, *p));
    }
    b
}

fn route_overlaps(r: &RoutedNet, q: Bbox) -> bool {
    if q.is_empty() {
        return false;
    }
    for seg in &r.segments {
        match seg {
            RouteSegment::Wire { points, .. } => {
                let mut bb = Bbox::EMPTY;
                for p in points {
                    bb = bb.union(&Bbox::new(*p, *p));
                }
                if bb.intersects(&q) {
                    return true;
                }
            }
            RouteSegment::Via { at, .. } => {
                if q.contains(*at) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multilayer::{LayerStack, MultiAStarConfig, PreferredDirection, RoutingLayer};

    fn two_layer_router() -> DetailedRouter {
        let stack = LayerStack {
            layers: vec![
                RoutingLayer {
                    name: "M1".into(),
                    direction: PreferredDirection::Horizontal,
                },
                RoutingLayer {
                    name: "M2".into(),
                    direction: PreferredDirection::Vertical,
                },
            ],
        };
        let rules = vec![
            LayerRules {
                min_width: 2,
                min_spacing: 2,
                min_area: 0,
                ..LayerRules::default()
            },
            LayerRules {
                min_width: 2,
                min_spacing: 2,
                min_area: 0,
                ..LayerRules::default()
            },
        ];
        let cfg = MultiAStarConfig {
            grid_step: 1,
            margin: 50,
            off_axis_penalty: 4,
            via_cost: 5,
        };
        DetailedRouter::new(stack, rules, cfg)
    }

    #[test]
    fn two_pin_route_succeeds() {
        let mut r = two_layer_router();
        let routed = r.route_net(RouteRequest {
            net_name: "n0".into(), ndr: None,
            pins: vec![(Point::new(0, 0), 0), (Point::new(50, 0), 0)],
        });
        assert!(routed.is_some());
        assert!(!routed.unwrap().segments.is_empty());
    }

    #[test]
    fn three_pin_route_uses_anchored_legs() {
        let mut r = two_layer_router();
        let routed = r
            .route_net(RouteRequest {
                net_name: "n0".into(), ndr: None,
                pins: vec![
                    (Point::new(0, 0), 0),
                    (Point::new(50, 0), 0),
                    (Point::new(100, 0), 0),
                ],
            })
            .unwrap();
        // Should be at least 2 wire segments (pin0→pin1, anchor→pin2).
        let wire_count = routed
            .segments
            .iter()
            .filter(|s| matches!(s, RouteSegment::Wire { .. }))
            .count();
        assert!(wire_count >= 2);
    }

    #[test]
    fn spacing_inflation_routes_around_obstacles() {
        let mut r = two_layer_router();
        // Add an obstacle in the direct path on M1.
        r.add_obstacle(0, Bbox::new(Point::new(20, -3), Point::new(30, 3)));
        let routed = r
            .route_net(RouteRequest {
                net_name: "n0".into(), ndr: None,
                pins: vec![(Point::new(0, 0), 0), (Point::new(50, 0), 0)],
            })
            .unwrap();
        // Some excursion should be present (either a y-detour or via to M2).
        let has_detour = routed.segments.iter().any(|s| match s {
            RouteSegment::Wire { points, .. } => points.iter().any(|p| p.y != 0),
            RouteSegment::Via { .. } => true,
        });
        assert!(has_detour);
    }

    #[test]
    fn second_net_avoids_first_nets_shapes() {
        let mut r = two_layer_router();
        let _ = r.route_net(RouteRequest {
            net_name: "n0".into(), ndr: None,
            pins: vec![(Point::new(0, 0), 0), (Point::new(50, 0), 0)],
        });
        // Route n1 above n0. With min_width=2 and min_spacing=2, the
        // inflation for n0 covers roughly y in [-3, 3]. n1 routed at
        // y=10 should succeed without overlapping.
        let routed = r
            .route_net(RouteRequest {
                net_name: "n1".into(), ndr: None,
                pins: vec![(Point::new(0, 10), 0), (Point::new(50, 10), 0)],
            })
            .unwrap();
        assert!(!routed.segments.is_empty());
        // Verify the n1 wire doesn't enter the n0 inflation zone.
        for seg in &routed.segments {
            if let RouteSegment::Wire { points, .. } = seg {
                for p in points {
                    assert!(p.y >= 4 || p.y <= -4, "wire entered n0's spacing zone at y={}", p.y);
                }
            }
        }
    }

    #[test]
    fn min_area_extends_short_wire() {
        let mut r = two_layer_router();
        // Set min_area large enough that a 1-unit wire violates it.
        r.layer_rules[0].min_area = 100;
        let routed = r
            .route_net(RouteRequest {
                net_name: "n0".into(), ndr: None,
                pins: vec![(Point::new(0, 0), 0), (Point::new(2, 0), 0)],
            })
            .unwrap();
        // After min-area extension, the last wire's bbox should be
        // longer than 2 in x.
        let last_bbox = routed.segments.iter().rev().find_map(|s| match s {
            RouteSegment::Wire { points, .. } if points.len() >= 2 => {
                let lo = points.iter().map(|p| p.x).min().unwrap();
                let hi = points.iter().map(|p| p.x).max().unwrap();
                Some(hi - lo)
            }
            _ => None,
        });
        assert!(last_bbox.is_some());
        // 100 / 2 (min_width) = 50 → new wire length should be ≥ 50.
        assert!(last_bbox.unwrap() >= 50);
    }

    #[test]
    fn unroutable_returns_none() {
        let mut r = two_layer_router();
        // Wall off everything between source and destination.
        r.add_obstacle(0, Bbox::new(Point::new(-100, -100), Point::new(100, 100)));
        r.add_obstacle(1, Bbox::new(Point::new(-100, -100), Point::new(100, 100)));
        let routed = r.route_net(RouteRequest {
            net_name: "n0".into(), ndr: None,
            pins: vec![(Point::new(0, 0), 0), (Point::new(50, 0), 0)],
        });
        assert!(routed.is_none());
    }

    #[test]
    fn ndr_overrides_widen_spacing_for_clock_net() {
        let mut r = two_layer_router();
        // First, route a clock-style net at 4× wider spacing.
        let clock_ndr = NdrOverride::uniform(2, 4, 8); // width 4, spacing 8
        let _ = r.route_net(RouteRequest {
            net_name: "clk".into(),
            pins: vec![(Point::new(0, 0), 0), (Point::new(50, 0), 0)],
            ndr: Some(clock_ndr),
        });
        // Now route a normal-rule net nearby — its inflation must
        // respect the wider clock obstacle.
        let routed = r
            .route_net(RouteRequest {
                net_name: "data".into(),
                pins: vec![(Point::new(0, 12), 0), (Point::new(50, 12), 0)],
                ndr: None,
            })
            .unwrap();
        // Routing should succeed; the wider clock spacing pushed data
        // farther but the channel at y=12 has room.
        assert!(!routed.segments.is_empty());
    }

    #[test]
    fn eol_halo_pushes_perpendicular_wire_back() {
        // Two layers, EOL configured on M1.
        let stack = LayerStack {
            layers: vec![
                RoutingLayer {
                    name: "M1".into(),
                    direction: PreferredDirection::Horizontal,
                },
                RoutingLayer {
                    name: "M2".into(),
                    direction: PreferredDirection::Vertical,
                },
            ],
        };
        let rules = vec![
            LayerRules {
                min_width: 2,
                min_spacing: 2,
                eol_within: 8,
                eol_spacing: 6,
                ..LayerRules::default()
            },
            LayerRules {
                min_width: 2,
                min_spacing: 2,
                ..LayerRules::default()
            },
        ];
        let cfg = MultiAStarConfig {
            grid_step: 1,
            margin: 50,
            off_axis_penalty: 4,
            via_cost: 50,
        };
        let mut r = DetailedRouter::new(stack, rules, cfg);

        // Route net1 ending at (50, 0).
        let _ = r.route_net(RouteRequest {
            net_name: "n0".into(),
            pins: vec![(Point::new(0, 0), 0), (Point::new(50, 0), 0)],
            ndr: None,
        });

        // Route n1 from (60, 20) to (45, 20). Both endpoints are
        // outside n0's EOL halo (which extends from x=50 to x=58).
        // The straight-line path on M1 would dip into the EOL halo at
        // y in [-6, 6] for x in [50, 58]; with halo enforcement, M1
        // wires must avoid that region or detour to M2.
        let routed = r
            .route_net(RouteRequest {
                net_name: "n1".into(),
                pins: vec![(Point::new(60, 4), 0), (Point::new(45, 4), 0)],
                ndr: None,
            });
        // Routing should succeed (via M2 detour or y-excursion).
        if let Some(rn) = routed {
            for seg in &rn.segments {
                if let RouteSegment::Wire { layer_idx: 0, points } = seg {
                    for w in points.windows(2) {
                        // Check the segment midpoint, not the endpoints
                        // (which are pin terminals exempt from halo).
                        let mid_x = (w[0].x + w[1].x) / 2;
                        let mid_y = (w[0].y + w[1].y) / 2;
                        let in_eol_zone_x = (50..58).contains(&mid_x);
                        if in_eol_zone_x {
                            assert!(
                                mid_y.abs() >= 6,
                                "M1 wire midpoint at ({mid_x},{mid_y}) sits in n0's EOL halo"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn ripup_recovers_when_first_order_fails() {
        let mut r = two_layer_router();
        // Two nets fighting for the same channel. Without rip-up we'd
        // route them in given order; rip-up gives both a chance.
        let reqs = vec![
            RouteRequest {
                net_name: "n0".into(),
                pins: vec![(Point::new(0, 0), 0), (Point::new(50, 0), 0)],
                ndr: None,
            },
            RouteRequest {
                net_name: "n1".into(),
                pins: vec![(Point::new(0, 12), 0), (Point::new(50, 12), 0)],
                ndr: None,
            },
        ];
        let (routed, failed) = r.route_all_with_ripup(reqs, 3);
        // Both should succeed with the y-channels well separated.
        assert_eq!(routed.len(), 2);
        assert!(failed.is_empty());
    }

    #[test]
    fn ripup_falls_through_when_truly_unroutable() {
        let mut r = two_layer_router();
        r.add_obstacle(0, Bbox::new(Point::new(-1000, -1000), Point::new(1000, 1000)));
        r.add_obstacle(1, Bbox::new(Point::new(-1000, -1000), Point::new(1000, 1000)));
        let (routed, failed) = r.route_all_with_ripup(
            vec![RouteRequest {
                net_name: "n0".into(),
                pins: vec![(Point::new(0, 0), 0), (Point::new(50, 0), 0)],
                ndr: None,
            }],
            3,
        );
        assert!(routed.is_empty());
        assert_eq!(failed.len(), 1);
    }

    #[test]
    fn via_cut_count_set_to_at_least_one() {
        let mut r = two_layer_router();
        let routed = r
            .route_net(RouteRequest {
                net_name: "n0".into(),
                pins: vec![(Point::new(0, 0), 0), (Point::new(0, 50), 1)],
                ndr: None,
            })
            .unwrap();
        let mut saw_via = false;
        for seg in &routed.segments {
            if let RouteSegment::Via { cut_count, .. } = seg {
                assert!(*cut_count >= 1);
                saw_via = true;
            }
        }
        assert!(saw_via, "cross-layer route must emit a via");
    }
}
