//! Density-fill insertion.
//!
//! Foundry density rules require, for every metal layer, that local
//! metal density falls in `[min_density, max_density]` over a
//! sliding window. Sparse regions get auto-filled with floating
//! "tile" polygons whose only purpose is to bring local density up.
//!
//! This module is the *generator*: given a `Region` of existing
//! metal and a window/tile spec, it returns a `Region` of fill
//! polygons to add. The check side lives in [`crate::rules::density_window`]
//! (KLayout-aligned tiling). This fill heuristic still walks the metal bbox
//! on a simple min-corner grid — it is not identical to the checker’s tile
//! origin, but suffices for v1 fill insertion.
//!
//! Algorithm:
//! 1. Walk the layout bbox in window-sized steps.
//! 2. For each window, compute local density. If below `min_density`,
//!    drop a regular grid of `tile_w × tile_h` fill polygons within
//!    the window, spaced by `tile_pitch`. Skip any tile that would
//!    overlap existing metal (with `keepout` clearance).
//! 3. Aggregate all tiles into the output region.
//!
//! Result is *un-merged* — each tile is its own polygon. Run
//! [`klayout_geom::merge`] downstream if a merged-fill output is
//! preferred.

use klayout_core::{Bbox, Point, Polygon};
use klayout_geom::{intersection, merge, Region};

#[derive(Clone, Debug)]
pub struct FillConfig {
    pub window: (i64, i64),
    pub step: (i64, i64),
    pub min_density: f64,
    /// Tile width × height (DBU).
    pub tile_size: (i64, i64),
    /// Pitch between adjacent tile centers in the fill grid.
    pub tile_pitch: (i64, i64),
    /// Minimum spacing from existing metal (used to inflate the
    /// "occupied" region before deciding where tiles can sit).
    pub keepout: i64,
}

/// Generate fill polygons that bring every low-density window up to
/// at least `min_density`.
pub fn generate_fill(metal: &Region, cfg: &FillConfig) -> Region {
    if cfg.window.0 <= 0
        || cfg.window.1 <= 0
        || cfg.tile_size.0 <= 0
        || cfg.tile_size.1 <= 0
    {
        return Region::empty();
    }
    let bbox = metal.bbox();
    if bbox.is_empty() {
        return Region::empty();
    }
    let merged = merge(metal);
    let window_area = (cfg.window.0 as f64) * (cfg.window.1 as f64);
    let tile_area = (cfg.tile_size.0 as f64) * (cfg.tile_size.1 as f64);

    // Inflate occupied metal by keepout for tile-overlap testing.
    let keepout_region = if cfg.keepout > 0 {
        klayout_geom::size(&merged, cfg.keepout, klayout_geom::SizeJoin::Miter)
    } else {
        merged.clone()
    };

    let step_x = cfg.step.0.max(1);
    let step_y = cfg.step.1.max(1);
    let mut all_fill: Vec<Polygon> = Vec::new();

    let mut y = bbox.min.y;
    while y < bbox.max.y {
        let mut x = bbox.min.x;
        while x < bbox.max.x {
            let win = Bbox::new(
                Point::new(x, y),
                Point::new(x + cfg.window.0, y + cfg.window.1),
            );
            // Compute current density in this window.
            let win_region = Region::from_polygons([Polygon::rect(win)]);
            let inter = intersection(&merged, &win_region);
            let area_in: i128 = inter.polygons().iter().map(polygon_area).sum();
            let cur_density = area_in as f64 / window_area;

            if cur_density < cfg.min_density {
                // Drop tiles in this window.
                let mut ty = y + cfg.tile_pitch.1 / 4;
                while ty + cfg.tile_size.1 <= y + cfg.window.1 {
                    let mut tx = x + cfg.tile_pitch.0 / 4;
                    while tx + cfg.tile_size.0 <= x + cfg.window.0 {
                        let tile_bbox = Bbox::new(
                            Point::new(tx, ty),
                            Point::new(tx + cfg.tile_size.0, ty + cfg.tile_size.1),
                        );
                        // Skip tiles that overlap keepout-inflated metal.
                        let tile_r = Region::from_polygons([Polygon::rect(tile_bbox)]);
                        let collision = intersection(&keepout_region, &tile_r);
                        if collision.is_empty() {
                            // Estimate added density; bail when sufficient.
                            all_fill.push(Polygon::rect(tile_bbox));
                            let added_area = (cfg.tile_size.0 * cfg.tile_size.1) as f64;
                            if (area_in as f64 + added_area * (all_fill.len() as f64))
                                / window_area
                                >= cfg.min_density
                            {
                                let _ = tile_area;
                                break;
                            }
                        }
                        tx += cfg.tile_pitch.0.max(1);
                    }
                    ty += cfg.tile_pitch.1.max(1);
                }
            }

            x += step_x;
        }
        y += step_y;
    }

    if all_fill.is_empty() {
        return Region::empty();
    }
    Region::from_polygons(all_fill)
}

fn polygon_area(p: &Polygon) -> i128 {
    let mut s: i128 = 0;
    let n = p.hull.len();
    if n < 3 {
        return 0;
    }
    for i in 0..n {
        let a = p.hull[i];
        let b = p.hull[(i + 1) % n];
        s += (a.x as i128) * (b.y as i128) - (b.x as i128) * (a.y as i128);
    }
    s.abs() / 2
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: i64, y0: i64, x1: i64, y1: i64) -> Polygon {
        Polygon::rect(Bbox::new(Point::new(x0, y0), Point::new(x1, y1)))
    }

    fn cfg() -> FillConfig {
        FillConfig {
            window: (100, 100),
            step: (100, 100),
            min_density: 0.30,
            tile_size: (5, 5),
            tile_pitch: (10, 10),
            keepout: 2,
        }
    }

    #[test]
    fn empty_metal_yields_no_fill() {
        let r = Region::empty();
        let fill = generate_fill(&r, &cfg());
        assert!(fill.is_empty());
    }

    #[test]
    fn fully_dense_window_gets_no_fill() {
        let r = Region::from_polygons([rect(0, 0, 100, 100)]);
        let fill = generate_fill(&r, &cfg());
        assert!(fill.is_empty(), "100% dense → no fill needed");
    }

    #[test]
    fn sparse_window_gets_fill() {
        // 10×10 metal in a 100×100 window = 1% density. min=30%.
        let r = Region::from_polygons([rect(0, 0, 10, 10)]);
        let fill = generate_fill(&r, &cfg());
        assert!(!fill.is_empty());
    }

    #[test]
    fn fill_avoids_existing_metal_via_keepout() {
        // Metal bbox 0..10. With keepout=2, fill tiles must be ≥ 2
        // away from x=10. Tile size 5, pitch 10: a tile starting at
        // x=12 doesn't overlap the keepout zone.
        let r = Region::from_polygons([rect(0, 0, 10, 100)]);
        let fill = generate_fill(&r, &cfg());
        for p in fill.polygons() {
            let bb = p.bbox();
            // Tile must not enter the inflated-metal zone (x in [-2..12]).
            assert!(bb.min.x >= 12, "tile bbox at x={} entered keepout", bb.min.x);
        }
    }
}
