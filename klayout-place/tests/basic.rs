//! Placement sanity tests.

use klayout_core::Point;
use klayout_place::*;

fn build_placement() -> Placement {
    let mut p = Placement::new();
    // 4 sites × 2 rows of width-1, height-1 cells.
    for j in 0..2 {
        p.add_row(Row {
            origin: Point::new(0, j),
            site_width: 1,
            num_sites: 4,
            height: 1,
        });
    }
    // 4 cells initially stacked at (0,0).
    for i in 0..4 {
        p.add_cell(Cell {
            name: format!("c{i}").into(),
            width: 1,
            height: 1,
            position: Point::new(0, 0),
            fixed: false,
        });
    }
    // Two simple nets.
    p.add_net(Net {
        name: "n0".into(),
        cell_indices: vec![0, 1],
        weight: 1.0,
    });
    p.add_net(Net {
        name: "n1".into(),
        cell_indices: vec![2, 3],
        weight: 1.0,
    });
    p
}

#[test]
fn hpwl_reduces_with_global_place() {
    let mut p = build_placement();
    // Force initial separation: spread cells around bbox.
    p.cells[0].position = Point::new(0, 0);
    p.cells[1].position = Point::new(3, 1);
    p.cells[2].position = Point::new(3, 0);
    p.cells[3].position = Point::new(0, 1);
    let before = p.hpwl();
    let cfg = global::GlobalConfig::default();
    global_place(&mut p, &cfg);
    let after = p.hpwl();
    assert!(after <= before, "global_place should not increase HPWL: {before} → {after}");
}

#[test]
fn legalize_snaps_to_row_grid() {
    let mut p = build_placement();
    // Place cells off-grid.
    p.cells[0].position = Point::new(0, 0);
    p.cells[1].position = Point::new(0, 0); // overlap with c0
    p.cells[2].position = Point::new(0, 1);
    p.cells[3].position = Point::new(2, 1);
    assert!(legalize(&mut p));
    // No two cells overlap on the same row after legalization.
    let mut by_row: std::collections::HashMap<i64, Vec<&Cell>> =
        std::collections::HashMap::new();
    for c in &p.cells {
        by_row.entry(c.position.y).or_default().push(c);
    }
    for (_, mut cells) in by_row {
        cells.sort_by_key(|c| c.position.x);
        for w in cells.windows(2) {
            assert!(w[0].position.x + w[0].width <= w[1].position.x);
        }
    }
}

#[test]
fn detailed_place_does_not_worsen_hpwl() {
    let mut p = build_placement();
    p.cells[0].position = Point::new(0, 0);
    p.cells[1].position = Point::new(3, 0);
    p.cells[2].position = Point::new(2, 0);
    p.cells[3].position = Point::new(1, 0);
    let cfg = detailed::DetailedConfig::default();
    let before = p.hpwl();
    detailed_place(&mut p, &cfg);
    let after = p.hpwl();
    assert!(after <= before);
}

#[test]
fn fixed_cells_dont_move() {
    let mut p = Placement::new();
    p.add_row(Row {
        origin: Point::new(0, 0),
        site_width: 1,
        num_sites: 100,
        height: 1,
    });
    p.add_cell(Cell {
        name: "fixed".into(),
        width: 1,
        height: 1,
        position: Point::new(50, 0),
        fixed: true,
    });
    p.add_cell(Cell {
        name: "free".into(),
        width: 1,
        height: 1,
        position: Point::new(0, 0),
        fixed: false,
    });
    p.add_net(Net {
        name: "n".into(),
        cell_indices: vec![0, 1],
        weight: 1.0,
    });
    global_place(&mut p, &global::GlobalConfig::default());
    // Fixed cell stays at x=50.
    assert_eq!(p.cells[0].position.x, 50);
    // Free cell pulled toward the fixed one.
    assert!(p.cells[1].position.x > 0);
}

#[test]
fn empty_placement_is_legalizable() {
    let mut p = Placement::new();
    p.add_row(Row {
        origin: Point::new(0, 0),
        site_width: 1,
        num_sites: 10,
        height: 1,
    });
    assert!(legalize(&mut p));
}

#[test]
fn full_pipeline_runs() {
    let mut p = build_placement();
    p.cells[0].position = Point::new(0, 0);
    p.cells[1].position = Point::new(3, 1);
    p.cells[2].position = Point::new(2, 0);
    p.cells[3].position = Point::new(1, 1);
    global_place(&mut p, &global::GlobalConfig::default());
    legalize(&mut p);
    detailed_place(&mut p, &detailed::DetailedConfig::default());
    // After full pipeline, every cell sits on row 0 or row 1.
    for c in &p.cells {
        assert!(c.position.y == 0 || c.position.y == 1);
    }
}

#[test]
fn multi_row_cell_reserves_space_on_both_rows() {
    // 2 rows of height 1, 10 sites wide each.
    let mut p = Placement::new();
    for j in 0..2 {
        p.add_row(Row {
            origin: Point::new(0, j),
            site_width: 1,
            num_sites: 10,
            height: 1,
        });
    }
    // Tall cell (height = 2 = two rows).
    p.add_cell(Cell {
        name: "tall".into(),
        width: 3,
        height: 2,
        position: Point::new(0, 0),
        fixed: false,
    });
    // Single-row cells.
    p.add_cell(Cell {
        name: "a".into(),
        width: 2,
        height: 1,
        position: Point::new(0, 0),
        fixed: false,
    });
    p.add_cell(Cell {
        name: "b".into(),
        width: 2,
        height: 1,
        position: Point::new(0, 1),
        fixed: false,
    });
    assert!(legalize(&mut p));
    // tall cell at row 0 occupies x in [tall.x, tall.x + 3] on both rows.
    let tall = &p.cells[0];
    let a = &p.cells[1];
    let b = &p.cells[2];
    // a (single-row, row 0) must not overlap tall in [tall.x, tall.x+3].
    assert!(a.position.x >= tall.position.x + tall.width || a.position.x + a.width <= tall.position.x);
    // b (single-row, row 1) likewise.
    assert!(b.position.x >= tall.position.x + tall.width || b.position.x + b.width <= tall.position.x);
}
