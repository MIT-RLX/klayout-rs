//! Fractional occupancy grid — soft congestion cost for path search.
//!
//! Each planar routing pass can **add** a smooth amount to cells the
//! centerline touches; A* / multi-layer A* add an integer piece
//! `weight × density` to the move cost so later nets steer around hot
//! regions without treating them as hard blockages.

use klayout_core::{Bbox, Point};

/// Row-major `density[gy * nx + gx]` in [0, ∞); values accumulate as nets
/// are routed.
#[derive(Clone, Debug)]
pub struct FractionalCongestionGrid {
    pub origin: Point,
    pub step: i64,
    pub nx: usize,
    pub ny: usize,
    pub density: Vec<f32>,
}

impl FractionalCongestionGrid {
    /// Empty grid covering `bbox` (expanded to whole steps from `origin`).
    pub fn for_bbox(bbox: Bbox, step: i64) -> Self {
        let step = step.max(1);
        let lo_x = (bbox.min.x / step) * step;
        let lo_y = (bbox.min.y / step) * step;
        let hi_x = ((bbox.max.x + step - 1) / step) * step;
        let hi_y = ((bbox.max.y + step - 1) / step) * step;
        let nx = (((hi_x - lo_x) / step).max(0) as usize) + 1;
        let ny = (((hi_y - lo_y) / step).max(0) as usize) + 1;
        Self {
            origin: Point::new(lo_x, lo_y),
            step,
            nx: nx.max(1),
            ny: ny.max(1),
            density: vec![0.0; nx.max(1) * ny.max(1)],
        }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.density.fill(0.0);
    }

    /// Nearest-cell lookup at world coordinates. Outside coverage → `0`.
    pub fn lookup_world(&self, p: Point) -> f32 {
        let gx_f = (p.x - self.origin.x) as f64 / self.step as f64;
        let gy_f = (p.y - self.origin.y) as f64 / self.step as f64;
        let gx = gx_f.floor() as isize;
        let gy = gy_f.floor() as isize;
        if gx < 0 || gy < 0 {
            return 0.0;
        }
        let gx = gx as usize;
        let gy = gy as usize;
        if gx >= self.nx || gy >= self.ny {
            return 0.0;
        }
        self.density[gy * self.nx + gx]
    }

    /// Add `delta` to every grid cell whose center falls on the closed axis-aligned
    /// segment between `a` and `b` (Manhattan centerlines only).
    pub fn add_manhattan_segment(&mut self, a: Point, b: Point, delta: f32) {
        if delta.abs() < f32::EPSILON {
            return;
        }
        if a.x == b.x {
            self.raster_vertical(a.x, a.y.min(b.y), a.y.max(b.y), delta);
        } else if a.y == b.y {
            self.raster_horizontal(a.y, a.x.min(b.x), a.x.max(b.x), delta);
        } else {
            // L-shape: split at elbow (matches typical Manhattan paths).
            let elbow = Point::new(b.x, a.y);
            self.add_manhattan_segment(a, elbow, delta);
            self.add_manhattan_segment(elbow, b, delta);
        }
    }

    /// Record a polyline path (consecutive Manhattan legs).
    pub fn add_manhattan_path(&mut self, pts: &[Point], delta: f32) {
        if pts.len() < 2 {
            return;
        }
        for w in pts.windows(2) {
            self.add_manhattan_segment(w[0], w[1], delta);
        }
    }

    fn raster_horizontal(&mut self, y: i64, x0: i64, x1: i64, delta: f32) {
        let step = self.step;
        let gy = ((y - self.origin.y) / step) as isize;
        if gy < 0 || gy as usize >= self.ny {
            return;
        }
        let gy = gy as usize;
        let mut gx0 = ((x0 - self.origin.x) / step) as isize;
        let mut gx1 = ((x1 - self.origin.x) / step) as isize;
        if gx0 > gx1 {
            std::mem::swap(&mut gx0, &mut gx1);
        }
        for gx in gx0..=gx1 {
            if gx < 0 {
                continue;
            }
            let gx = gx as usize;
            if gx >= self.nx {
                continue;
            }
            let i = gy * self.nx + gx;
            self.density[i] += delta;
        }
    }

    fn raster_vertical(&mut self, x: i64, y0: i64, y1: i64, delta: f32) {
        let step = self.step;
        let gx = ((x - self.origin.x) / step) as isize;
        if gx < 0 || gx as usize >= self.nx {
            return;
        }
        let gx = gx as usize;
        let mut gy0 = ((y0 - self.origin.y) / step) as isize;
        let mut gy1 = ((y1 - self.origin.y) / step) as isize;
        if gy0 > gy1 {
            std::mem::swap(&mut gy0, &mut gy1);
        }
        for gy in gy0..=gy1 {
            if gy < 0 {
                continue;
            }
            let gy = gy as usize;
            if gy >= self.ny {
                continue;
            }
            let i = gy * self.nx + gx;
            self.density[i] += delta;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_increments_cells() {
        let mut g = FractionalCongestionGrid::for_bbox(
            Bbox::new(Point::new(0, 0), Point::new(1000, 1000)),
            100,
        );
        g.add_manhattan_segment(Point::new(0, 0), Point::new(500, 0), 0.5);
        assert!(g.density.iter().any(|&v| v > 0.0));
        let s: f32 = g.density.iter().sum();
        assert!(s > 1.0);
    }
}
