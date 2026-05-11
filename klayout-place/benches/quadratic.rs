//! Criterion benchmarks for [`klayout_place::quadratic_place`] (B2B CG solver).

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use klayout_core::Point;
use klayout_place::{quadratic_place, Cell, Net, Placement, QuadraticConfig, Row};

/// Linear chain of `n_cells` movable cells (`n_cells − 1` two-pin nets).
fn build_placement_chain(n_cells: usize) -> Placement {
    let mut p = Placement::new();
    let span = ((n_cells as i64).saturating_mul(8)).clamp(96, 20_000);
    p.add_row(Row {
        origin: Point::new(0, 0),
        site_width: 1,
        num_sites: span as u32,
        height: 1,
    });
    for i in 0..n_cells {
        let x = (i.wrapping_mul(17)) as i64 % span.max(1);
        p.add_cell(Cell {
            name: format!("c{i}").into(),
            width: 1,
            height: 1,
            position: Point::new(x, 0),
            fixed: false,
        });
    }
    for i in 0..n_cells.saturating_sub(1) {
        p.add_net(Net {
            name: format!("n{i}").into(),
            cell_indices: vec![i, i + 1],
            weight: 1.0,
        });
    }
    p
}

fn bench_quadratic(c: &mut Criterion) {
    let mut g = c.benchmark_group("quadratic");
    for &n_cells in &[8usize, 32, 128] {
        let template = build_placement_chain(n_cells);
        g.bench_with_input(BenchmarkId::from_parameter(n_cells), &template, |b, tpl| {
            let cf = QuadraticConfig::default();
            b.iter(|| {
                let mut work = tpl.clone();
                quadratic_place(black_box(&mut work), black_box(&cf));
            });
        });
    }
    g.finish();
}

criterion_group!(benches, bench_quadratic);
criterion_main!(benches);
