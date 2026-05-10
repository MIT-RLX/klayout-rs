//! RSMT benchmark: synthetic ISPD-style pin sets.
//!
//! Goal: track wall time and total wirelength across pin counts that
//! cover the realistic spectrum — 4 (typical clock-domain net), 16
//! (typical bus), 64 (high-fanout signal), 256 (clock distribution).
//! At 256 pins the `O(n⁵)` Iterated 1-Steiner becomes the bottleneck;
//! that's the point at which a FLUTE-style lookup-table flow earns
//! its keep, and this benchmark is what would surface the regression.

use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};
use klayout_core::Point;
use klayout_route::rsmt;

fn deterministic_pins(n: usize, seed: u64) -> Vec<Point> {
    // Linear-congruential RNG so the same `n` always yields the same
    // pin layout — benchmark-to-benchmark numbers are comparable.
    let mut s = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let x = (s >> 32) as u32 % 10_000;
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let y = (s >> 32) as u32 % 10_000;
        out.push(Point::new(x as i64, y as i64));
    }
    out
}

fn rsmt_bench(c: &mut Criterion) {
    let mut g = c.benchmark_group("rsmt");
    for &n in &[4usize, 16, 64, 256] {
        let pins = deterministic_pins(n, 0xC0FFEE);
        g.throughput(Throughput::Elements(n as u64));
        g.bench_with_input(BenchmarkId::from_parameter(n), &pins, |b, pins| {
            b.iter(|| rsmt(black_box(pins)))
        });
    }
    g.finish();
}

criterion_group!(benches, rsmt_bench);
criterion_main!(benches);
