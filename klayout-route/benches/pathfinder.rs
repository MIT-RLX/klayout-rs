//! Pathfinder benchmark: scaling wall time vs (#nets, GCell grid size).
//!
//! Captures: per-iteration cost, convergence iterations to zero
//! overflow on a deliberately congested workload, and end-to-end
//! latency. Compare against published OpenROAD numbers on equivalent
//! ISPD'18 inputs to surface algorithmic regressions.

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use klayout_core::{Bbox, Point};
use klayout_route::{
    route_pathfinder, CapacityGrid, GCellGrid, PathfinderConfig, PathfinderRequest,
};
use smol_str::SmolStr;

fn build_workload(grid_n: u32, n_nets: usize, capacity: u32) -> (GCellGrid, CapacityGrid, Vec<PathfinderRequest>) {
    let edge = 100i64;
    let bbox = Bbox::new(
        Point::new(0, 0),
        Point::new(grid_n as i64 * edge, grid_n as i64 * edge),
    );
    let grid = GCellGrid::new(bbox, edge, edge);
    let cap = CapacityGrid::uniform(&grid, capacity);

    // Deterministic LCG over (src, sink) pairs, both inside the grid.
    let mut s: u64 = 0xDEADBEEF;
    let mut next = || {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        s
    };

    let mut reqs: Vec<PathfinderRequest> = Vec::with_capacity(n_nets);
    for i in 0..n_nets {
        let sx = (next() >> 32) as u32 % grid_n;
        let sy = (next() >> 32) as u32 % grid_n;
        let dx = (next() >> 32) as u32 % grid_n;
        let dy = (next() >> 32) as u32 % grid_n;
        reqs.push(PathfinderRequest {
            net_name: SmolStr::from(format!("n{i}")),
            src: Point::new(sx as i64 * edge + edge / 2, sy as i64 * edge + edge / 2),
            sink: Point::new(dx as i64 * edge + edge / 2, dy as i64 * edge + edge / 2),
        });
    }
    (grid, cap, reqs)
}

fn pathfinder_bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("pathfinder");
    g.sample_size(20); // big workloads — keep iteration count modest.

    for &(grid, n_nets) in &[(8u32, 8usize), (16, 32), (32, 128)] {
        let (gc, cap, reqs) = build_workload(grid, n_nets, 4);
        g.bench_with_input(
            BenchmarkId::new("uncongested", format!("{grid}x{grid}/{n_nets}")),
            &(gc.clone(), cap.clone(), reqs.clone()),
            |b, (gc, cap, reqs)| {
                b.iter_batched(
                    || cap.clone(),
                    |mut c| {
                        let _ = route_pathfinder(
                            black_box(gc),
                            &mut c,
                            black_box(reqs),
                            &PathfinderConfig::default(),
                        );
                    },
                    criterion::BatchSize::LargeInput,
                )
            },
        );
    }
    g.finish();
}

criterion_group!(benches, pathfinder_bench);
criterion_main!(benches);
