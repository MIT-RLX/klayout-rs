//! DRC `width` benchmark — wall time as a function of polygon count
//! and polygon edge count.
//!
//! Specifically targets the indexed-edge path
//! (`width_pairs_indexed`); the asymptotic improvement over the
//! naive `O(E²)` per-polygon enumeration only matters above ~64
//! edges per polygon, so this bench includes a "fractured polygon"
//! case (many small jogs) at the high edge-count end.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use klayout_core::{Bbox, Point, Polygon};
use klayout_drc::width;
use klayout_geom::Region;

/// Build a region of `n` rectangles laid out on a grid, each
/// `wide_dbu × wide_dbu` and spaced by `spacing_dbu`. Width violations
/// occur whenever `wide_dbu < min`; tune to land on either side.
fn rect_grid(n: usize, wide_dbu: i64, spacing_dbu: i64) -> Region {
    let cols = (n as f64).sqrt().ceil() as usize;
    let mut polys: Vec<Polygon> = Vec::with_capacity(n);
    for i in 0..n {
        let r = i / cols;
        let c = i % cols;
        let x0 = c as i64 * (wide_dbu + spacing_dbu);
        let y0 = r as i64 * (wide_dbu + spacing_dbu);
        let x1 = x0 + wide_dbu;
        let y1 = y0 + wide_dbu;
        polys.push(Polygon::rect(Bbox::new(Point::new(x0, y0), Point::new(x1, y1))));
    }
    Region::from_polygons(polys)
}

/// Build a single highly-fractured polygon (one big shape with many
/// stair-step jogs). Exercises the per-polygon edge-bucket index.
fn jagged_polygon(n_jogs: usize, jog_dbu: i64) -> Region {
    let mut hull = Vec::with_capacity(n_jogs * 2 + 4);
    hull.push(Point::new(0, 0));
    for i in 0..n_jogs {
        hull.push(Point::new((i as i64 + 1) * jog_dbu, 0));
        hull.push(Point::new((i as i64 + 1) * jog_dbu, jog_dbu));
    }
    hull.push(Point::new(n_jogs as i64 * jog_dbu, 1000));
    hull.push(Point::new(0, 1000));
    Region::from_polygons(vec![Polygon::from_hull(hull)])
}

fn drc_width_bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("drc_width");
    g.sample_size(20);

    for &n in &[16usize, 256, 1024] {
        // No violation: each polygon is 100 DBU wide, min is 50.
        let r_clean = rect_grid(n, 100, 50);
        g.bench_with_input(BenchmarkId::new("grid_clean", n), &r_clean, |b, r| {
            b.iter(|| width(black_box(r), 50))
        });

        // All violation: each polygon is 30 DBU wide, min is 50.
        let r_all = rect_grid(n, 30, 50);
        g.bench_with_input(BenchmarkId::new("grid_all_viol", n), &r_all, |b, r| {
            b.iter(|| width(black_box(r), 50))
        });
    }

    for &n_jogs in &[4usize, 32, 128] {
        let r = jagged_polygon(n_jogs, 100);
        g.bench_with_input(
            BenchmarkId::new("jagged_polygon", n_jogs),
            &r,
            |b, r| b.iter(|| width(black_box(r), 50)),
        );
    }
    g.finish();
}

criterion_group!(benches, drc_width_bench);
criterion_main!(benches);
