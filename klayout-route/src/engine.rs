//! Pluggable router engine abstraction.
//!
//! The existing [`Planner`] trait covers single-segment 2-pin
//! routing (port → port → centerline path). [`RouterEngine`] is the
//! batch-routing analogue: given many `RouteRequest`s and a global
//! obstacle/capacity model, produce many `RoutedNet`s. Lets callers
//! swap an A* implementation for an ILP, MCF, or pattern-based
//! engine without changing call sites.
//!
//! v1 ships:
//! * [`MultilayerEngine`] — wraps the existing 3D-A* multilayer
//!   router (one net at a time, no global routing).
//! * [`DetailedEngine`] — wraps [`crate::DetailedRouter`] (rule-aware
//!   spacing, NDR, antenna, rip-up).
//! * [`GlobalThenDetailedEngine`] — runs global routing first to
//!   plan corridors, then hands off to detailed.
//!
//! All return `Vec<RoutedNet>` so a downstream stylizer / SPEF
//! emitter sees uniform output.

use crate::detailed::{DetailedRouter, RouteRequest, RoutedNet};
use crate::global::{
    route_global, CapacityGrid, GCellGrid, GlobalRouteRequest, GlobalRouterConfig,
};
use crate::multilayer::{multilayer_route, LayerStack, MultiAStarConfig, RouteSegment};
use crate::planner::Obstacles;
use smol_str::SmolStr;

pub trait RouterEngine {
    fn route_batch(&mut self, requests: Vec<RouteRequest>) -> Vec<RoutedNet>;
}

/// Wraps `multilayer_route` for trait-object use. One net at a time;
/// no per-net obstacle update (caller supplies a fixed obstacle map).
pub struct MultilayerEngine {
    pub stack: LayerStack,
    pub obstacles: Vec<Obstacles>,
    pub config: MultiAStarConfig,
}

impl RouterEngine for MultilayerEngine {
    fn route_batch(&mut self, requests: Vec<RouteRequest>) -> Vec<RoutedNet> {
        let mut out = Vec::with_capacity(requests.len());
        for req in requests {
            if req.pins.len() < 2 {
                continue;
            }
            let mut segs: Vec<RouteSegment> = Vec::new();
            let first = multilayer_route(
                req.pins[0],
                req.pins[1],
                &self.obstacles,
                &self.stack,
                &self.config,
            );
            if let Some(s) = first {
                segs.extend(s);
            } else {
                continue;
            }
            let mut ok = true;
            for &pin in &req.pins[2..] {
                let leg = multilayer_route(
                    req.pins[0],
                    pin,
                    &self.obstacles,
                    &self.stack,
                    &self.config,
                );
                match leg {
                    Some(s) => segs.extend(s),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                out.push(RoutedNet {
                    net_name: req.net_name,
                    segments: segs,
                });
            }
        }
        out
    }
}

pub struct DetailedEngine {
    pub router: DetailedRouter,
}

impl RouterEngine for DetailedEngine {
    fn route_batch(&mut self, requests: Vec<RouteRequest>) -> Vec<RoutedNet> {
        self.router.route_all(requests)
    }
}

/// Two-stage flow: global router places each net into a corridor of
/// gcells, then the detailed router routes within those corridors.
/// The corridor is informational in v1 (we don't yet bias the
/// detailed router toward in-corridor cells), but the global pass
/// catches infeasible designs before we burn detailed-router time.
pub struct GlobalThenDetailedEngine {
    pub grid: GCellGrid,
    pub capacity: CapacityGrid,
    pub global_config: GlobalRouterConfig,
    pub detailed: DetailedRouter,
}

impl RouterEngine for GlobalThenDetailedEngine {
    fn route_batch(&mut self, requests: Vec<RouteRequest>) -> Vec<RoutedNet> {
        // Stage 1: global routing predicts corridors per net.
        let global_reqs: Vec<GlobalRouteRequest> = requests
            .iter()
            .map(|r| GlobalRouteRequest {
                net_name: r.net_name.clone(),
                src: r.pins.first().copied().map(|(p, _)| p).unwrap_or_default(),
                dst: r
                    .pins
                    .get(1)
                    .copied()
                    .map(|(p, _)| p)
                    .unwrap_or_default(),
            })
            .collect();
        let global_results = route_global(
            &self.grid,
            &mut self.capacity,
            &global_reqs,
            &self.global_config,
        );
        // Skip nets the global router declared infeasible.
        let mut routable: Vec<RouteRequest> = Vec::new();
        for (req, gres) in requests.into_iter().zip(global_results.iter()) {
            if !gres.gcells.is_empty() {
                routable.push(req);
            }
        }
        // Stage 2: hand off to detailed router.
        self.detailed.route_all(routable)
    }
}

/// Convenience: name-sort a routed batch (useful for deterministic
/// output regardless of engine internal order).
pub fn sort_routed_by_name(nets: &mut [RoutedNet]) {
    nets.sort_by(|a, b| a.net_name.cmp(&b.net_name));
}

#[derive(Clone, Debug, Default)]
pub struct EngineMetrics {
    pub routed: usize,
    pub failed: usize,
    pub names_failed: Vec<SmolStr>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detailed::LayerRules;
    use crate::multilayer::{LayerStack, PreferredDirection, RoutingLayer};
    use klayout_core::Point;

    fn two_layer_stack() -> LayerStack {
        LayerStack {
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
        }
    }

    #[test]
    fn multilayer_engine_routes_a_pair() {
        let mut engine = MultilayerEngine {
            stack: two_layer_stack(),
            obstacles: vec![Obstacles::default(), Obstacles::default()],
            config: MultiAStarConfig::default(),
        };
        let routed = engine.route_batch(vec![RouteRequest {
            net_name: "n0".into(),
            pins: vec![(Point::new(0, 0), 0), (Point::new(50, 0), 0)],
            ndr: None,
        }]);
        assert_eq!(routed.len(), 1);
    }

    #[test]
    fn detailed_engine_wraps_router() {
        let stack = two_layer_stack();
        let rules = vec![LayerRules::default(), LayerRules::default()];
        let cfg = MultiAStarConfig::default();
        let mut engine = DetailedEngine {
            router: DetailedRouter::new(stack, rules, cfg),
        };
        let routed = engine.route_batch(vec![RouteRequest {
            net_name: "a".into(),
            pins: vec![(Point::new(0, 0), 0), (Point::new(20, 0), 0)],
            ndr: None,
        }]);
        assert_eq!(routed.len(), 1);
    }

    #[test]
    fn engines_swap_via_trait_object() {
        let stack = two_layer_stack();
        let rules = vec![LayerRules::default(), LayerRules::default()];
        let cfg = MultiAStarConfig::default();
        let engines: Vec<Box<dyn RouterEngine>> = vec![
            Box::new(MultilayerEngine {
                stack: stack.clone(),
                obstacles: vec![Obstacles::default(), Obstacles::default()],
                config: cfg.clone(),
            }),
            Box::new(DetailedEngine {
                router: DetailedRouter::new(stack, rules, cfg),
            }),
        ];
        let req = RouteRequest {
            net_name: "n".into(),
            pins: vec![(Point::new(0, 0), 0), (Point::new(20, 0), 0)],
            ndr: None,
        };
        for mut engine in engines {
            let routed = engine.route_batch(vec![req.clone()]);
            assert_eq!(routed.len(), 1);
        }
    }
}
