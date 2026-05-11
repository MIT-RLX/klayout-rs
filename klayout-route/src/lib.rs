//! `klayout-route` — routing primitives.
//!
//! Three orthogonal stages, expressed as traits so each can be swapped:
//!
//! * [`Planner`] — given two `Port`s and an obstacle environment, produce
//!   a centerline `Path` connecting them. v1 ships a 90°-manhattan
//!   planner that drops one bend.
//! * [`Stylizer`] — turn a centerline `Path` into shapes (or instances of
//!   bend/straight cells). v1 emits a single `Path` shape on the port's
//!   layer.
//! * [`Bundler`] — given a set of source/target port pairs, produce
//!   length-matched / non-crossing `Path`s. v1 is identity (no bundling).
//!
//! Each trait returns simple data; composition is the user's job. This
//! keeps the surface flexible and testable without forcing one routing
//! style on every PDK.

pub mod antenna;
pub mod astar;
pub mod bundler;
pub mod crosstalk;
pub mod congestion;
pub mod detailed;
pub mod engine;
pub mod global;
pub mod metrics;
pub mod multilayer;
pub mod ordering;
pub mod planner;
pub mod em;
pub mod ir_drop;
pub mod pathfinder;
pub mod pathfinder_multi;
pub mod power_grid;
pub mod rsmt;
pub mod stylizer;
pub mod track;

pub use antenna::{antenna_check, antenna_fix, AntennaRules, AntennaViolation, GatePin};
pub use astar::AStarPlanner;
pub use bundler::{Bundler, DiffPairBundler, IdentityBundler, LengthMatchedBundler};
pub use crosstalk::{
    detect_violations, insert_shielding, CrosstalkConfig, CrosstalkViolation,
};
pub use congestion::FractionalCongestionGrid;
pub use detailed::{DetailedRouter, LayerRules, NdrOverride, RouteRequest, RoutedNet};
pub use engine::{
    sort_routed_by_name, DetailedEngine, EngineMetrics, GlobalThenDetailedEngine,
    MultilayerEngine, RouterEngine,
};
pub use global::{
    route_global, CapacityGrid, GCellGrid, GCellId, GlobalRoute, GlobalRouteRequest,
    GlobalRouterConfig,
};
pub use metrics::{compute_metrics, summarize_congestion, CongestionSummary, NetMetrics, RoutingMetrics};
pub use ordering::{
    criticality_descending, fanout, fanout_descending, hpwl, hpwl_ascending, hpwl_descending,
    reorder,
};
pub use multilayer::{
    multilayer_route, multilayer_route_with_congestion, LayerStack, MultiAStarConfig,
    PreferredDirection, RouteSegment, RoutingLayer,
};
pub use planner::{ManhattanPlanner, Obstacles, Planner};
pub use em::{black_mttf_years, em_check, EmLayer, EmViolation, NetCurrent};
pub use ir_drop::{
    build_power_grid_ir, solve_ir_drop, IrDropConfig, IrDropResult, PowerGridIr, PowerNode,
    PowerSegment,
};
pub use pathfinder::{
    route_pathfinder, PathfinderConfig, PathfinderRequest, PathfinderResult, PathfinderRoute,
};
pub use pathfinder_multi::{
    route_multi_pathfinder, LayerDirection, LayerSpec, MultiConfig, MultiLayerStack,
    MultiPathRequest, MultiPathRoute, MultiResult,
};
pub use power_grid::{generate_power_grid, PowerGrid, PowerGridConfig, RingConfig};
pub use rsmt::{rsmt, rsmt_fast, RsmtTree};
pub use stylizer::{routed_to_shapes, RoutedShapeParams, Stylizer, WirePathStylizer};
pub use track::{snap_routed_net, TrackGrid, TrackGrids};
