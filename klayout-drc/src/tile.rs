//! Tile-based DRC.
//!
//! Decompose a layout into a grid of tiles (each tile inflated by a
//! halo equal to the rule constant) and run a rule on each tile in
//! parallel. The halo guarantees that any violation crossing tile
//! boundaries is discovered by *some* tile that contains both
//! contributors. After per-tile execution, results are clipped back to
//! the un-inflated tile bounds to avoid double-counting in halo
//! overlaps, then merged with the geom layer.
//!
//! This module composes with the existing rule kernels — a tile
//! "executes" a closure that takes a `Region` and returns a `Region`.
//! Use [`tile_unary`] for `width`/`area_min` (single input layer) and
//! [`tile_binary`] for `space`/`separation`/`overlap`/`enclosing`
//! (two input layers).
//!
//! Sizing notes:
//! * Halo must be ≥ the rule constant. Most rules in `rules.rs`
//!   examine pairs of edges within `min` distance, so a halo of `min`
//!   is sufficient.
//! * Tile size is a tuning parameter — larger tiles do less merging
//!   work but extract less parallelism. A reasonable default is
//!   `max(layout_bbox / 8, 100µm)`.

use klayout_core::{Bbox, Point, Polygon};
use klayout_geom::{intersection, merge, Region};
use rayon::prelude::*;

#[derive(Copy, Clone, Debug)]
pub struct TileConfig {
    /// Tile size in DBU. The layout bbox is partitioned into a regular
    /// grid of `tile_size.0 × tile_size.1` cells.
    pub tile_size: (i64, i64),
    /// Halo distance in DBU. Each tile is inflated by this amount in
    /// every direction before running the rule kernel.
    pub halo: i64,
}

impl TileConfig {
    /// Pick a tile size that yields ~`target_tiles` tiles total over
    /// the given bbox.
    pub fn auto(bbox: Bbox, halo: i64, target_tiles: u32) -> Self {
        let target = target_tiles.max(1) as f64;
        let side = (target.sqrt()).max(1.0) as i64;
        let tile_w = (bbox.width().max(1) / side).max(halo * 2 + 1);
        let tile_h = (bbox.height().max(1) / side).max(halo * 2 + 1);
        Self {
            tile_size: (tile_w, tile_h),
            halo,
        }
    }
}

/// Run `kernel` on each tile of `r`. Each tile sees `r` clipped to
/// the tile's *inflated* bounds; the kernel's output is clipped to
/// the *un-inflated* bounds before merging across tiles.
pub fn tile_unary<F>(r: &Region, cfg: TileConfig, kernel: F) -> Region
where
    F: Fn(&Region) -> Region + Sync,
{
    let bbox = r.bbox();
    if bbox.is_empty() {
        return Region::empty();
    }
    let tiles = compute_tiles(bbox, cfg);
    let polys: Vec<Polygon> = tiles
        .par_iter()
        .flat_map_iter(|tile| {
            let halo_region = clip_to_bbox(r, tile.inflated);
            let result = kernel(&halo_region);
            let clipped = clip_to_bbox(&result, tile.core);
            clipped.polygons().to_vec()
        })
        .collect();
    if polys.is_empty() {
        return Region::empty();
    }
    merge(&Region::from_polygons(polys))
}

/// Two-input variant — `kernel` takes `(&Region, &Region)`.
pub fn tile_binary<F>(a: &Region, b: &Region, cfg: TileConfig, kernel: F) -> Region
where
    F: Fn(&Region, &Region) -> Region + Sync,
{
    let bbox = a.bbox().union(&b.bbox());
    if bbox.is_empty() {
        return Region::empty();
    }
    let tiles = compute_tiles(bbox, cfg);
    let polys: Vec<Polygon> = tiles
        .par_iter()
        .flat_map_iter(|tile| {
            let a_t = clip_to_bbox(a, tile.inflated);
            let b_t = clip_to_bbox(b, tile.inflated);
            let result = kernel(&a_t, &b_t);
            let clipped = clip_to_bbox(&result, tile.core);
            clipped.polygons().to_vec()
        })
        .collect();
    if polys.is_empty() {
        return Region::empty();
    }
    merge(&Region::from_polygons(polys))
}

#[derive(Copy, Clone)]
struct Tile {
    /// Un-inflated tile bounds — used to clip results so halo overlaps
    /// don't double-count.
    core: Bbox,
    /// Inflated tile bounds — what the kernel sees.
    inflated: Bbox,
}

fn compute_tiles(bbox: Bbox, cfg: TileConfig) -> Vec<Tile> {
    let (tw, th) = cfg.tile_size;
    let halo = cfg.halo.max(0);
    let tw = tw.max(1);
    let th = th.max(1);
    let mut tiles = Vec::new();
    let mut y = bbox.min.y;
    while y < bbox.max.y {
        let y2 = (y + th).min(bbox.max.y);
        let mut x = bbox.min.x;
        while x < bbox.max.x {
            let x2 = (x + tw).min(bbox.max.x);
            let core = Bbox::new(Point::new(x, y), Point::new(x2, y2));
            let inflated = Bbox::new(
                Point::new(x - halo, y - halo),
                Point::new(x2 + halo, y2 + halo),
            );
            tiles.push(Tile { core, inflated });
            x = x2;
        }
        y = y2;
    }
    tiles
}

fn clip_to_bbox(r: &Region, b: Bbox) -> Region {
    if r.is_empty() || b.is_empty() {
        return Region::empty();
    }
    intersection(r, &Region::from_polygons([Polygon::rect(b)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{space, width};

    #[test]
    fn tile_width_matches_global_width() {
        let polys = vec![
            Polygon::rect(Bbox::new(Point::new(0, 0), Point::new(50, 5))),
            Polygon::rect(Bbox::new(Point::new(100, 0), Point::new(150, 5))),
        ];
        let r = Region::from_polygons(polys);
        let global = width(&r, 10);
        let cfg = TileConfig {
            tile_size: (60, 60),
            halo: 10,
        };
        let tiled = tile_unary(&r, cfg, |x| width(x, 10));
        assert_eq!(global.len(), tiled.len());
    }

    #[test]
    fn tile_space_catches_cross_tile_violations() {
        // Two boxes 5 units apart, on either side of the tile boundary.
        let polys = vec![
            Polygon::rect(Bbox::new(Point::new(0, 0), Point::new(50, 50))),
            Polygon::rect(Bbox::new(Point::new(55, 0), Point::new(100, 50))),
        ];
        let r = Region::from_polygons(polys);
        let cfg = TileConfig {
            tile_size: (50, 50),
            halo: 10,
        };
        let global = space(&r, 10);
        let tiled = tile_unary(&r, cfg, |x| space(x, 10));
        assert_eq!(global.is_empty(), tiled.is_empty());
        if !global.is_empty() {
            assert!(!tiled.is_empty(), "halo should catch cross-tile violation");
        }
    }

    #[test]
    fn empty_input_yields_empty_output() {
        let r = Region::empty();
        let cfg = TileConfig {
            tile_size: (100, 100),
            halo: 10,
        };
        let out = tile_unary(&r, cfg, |x| width(x, 10));
        assert!(out.is_empty());
    }
}
