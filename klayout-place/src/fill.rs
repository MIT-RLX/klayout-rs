//! Tap and decap insertion — fill standard-cell rows with non-signal
//! cells the design needs but the placer doesn't produce.
//!
//! * **Tap cells** — well taps for substrate / N-well biasing. Foundry
//!   rules typically require one every `tap_pitch` DBU along each row.
//! * **Decap cells** — decoupling capacitors that fill empty placement
//!   gaps to provide local charge for switching events.
//!
//! Both run *after* legalization on a `Placement`. They insert new
//! `Cell`s into the placement's `cells` vector with `fixed=true`.

use crate::types::{Cell, Placement};
use klayout_core::Point;
use smol_str::SmolStr;

/// Insert tap cells of width `tap_width` and height matching the row,
/// spaced approximately `tap_pitch` apart along each row. Tap cells
/// nudge into the next free site if the periodic position is
/// occupied. The cell name is `tap_<row>_<idx>` for diagnostics.
pub fn insert_taps(p: &mut Placement, tap_width: i64, tap_pitch: i64, tap_kind: TapKind) -> usize {
    let mut inserted = 0;
    for (ri, row) in p.rows.clone().iter().enumerate() {
        // Build occupancy mask: which x ranges are taken in this row.
        let occupied: Vec<(i64, i64)> = p
            .cells
            .iter()
            .filter(|c| c.position.y == row.origin.y && !c.name.starts_with("tap_"))
            .map(|c| (c.position.x, c.position.x + c.width))
            .collect();

        let mut x = row.origin.x;
        let mut k = 0;
        while x + tap_width <= row.x_max() {
            // Find nearest unoccupied site at or after x.
            let mut place_x = row.snap_x(x);
            while occupied.iter().any(|(lo, hi)| place_x < *hi && place_x + tap_width > *lo) {
                place_x += row.site_width;
                if place_x + tap_width > row.x_max() {
                    place_x = -1;
                    break;
                }
            }
            if place_x >= 0 {
                p.cells.push(Cell {
                    name: SmolStr::from(format!("tap_{ri}_{k}")),
                    width: tap_width,
                    height: row.height,
                    position: Point::new(place_x, row.origin.y),
                    fixed: true,
                });
                inserted += 1;
                k += 1;
                x = place_x + tap_pitch;
            } else {
                break;
            }
        }
    }
    let _ = tap_kind;
    inserted
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TapKind {
    Nwell,
    Pwell,
    Both,
}

/// Greedy decap insertion: scan each row, find every gap between
/// consecutive non-fixed cells, and fill with the largest decap whose
/// width fits.
pub fn insert_decaps(p: &mut Placement, decap_widths: &[i64]) -> usize {
    let mut decaps_sorted: Vec<i64> = decap_widths.to_vec();
    decaps_sorted.sort_unstable_by(|a, b| b.cmp(a)); // largest first
    if decaps_sorted.is_empty() {
        return 0;
    }
    let mut inserted = 0;
    for (ri, row) in p.rows.clone().iter().enumerate() {
        let mut row_cells: Vec<(i64, i64)> = p
            .cells
            .iter()
            .filter(|c| c.position.y == row.origin.y)
            .map(|c| (c.position.x, c.position.x + c.width))
            .collect();
        row_cells.sort_by_key(|(lo, _)| *lo);

        let mut frontier_x = row.origin.x;
        let mut k = 0;
        for (lo, hi) in row_cells.iter() {
            if *lo > frontier_x {
                fill_gap(
                    p,
                    ri,
                    row,
                    frontier_x,
                    *lo,
                    &decaps_sorted,
                    &mut k,
                    &mut inserted,
                );
            }
            frontier_x = (*hi).max(frontier_x);
        }
        // Tail gap.
        if frontier_x < row.x_max() {
            fill_gap(
                p,
                ri,
                row,
                frontier_x,
                row.x_max(),
                &decaps_sorted,
                &mut k,
                &mut inserted,
            );
        }
    }
    inserted
}

#[allow(clippy::too_many_arguments)]
fn fill_gap(
    p: &mut Placement,
    ri: usize,
    row: &crate::types::Row,
    lo: i64,
    hi: i64,
    decap_widths: &[i64],
    k: &mut usize,
    inserted: &mut usize,
) {
    let mut x = lo;
    while x < hi {
        let remaining = hi - x;
        let chosen = decap_widths.iter().copied().find(|&w| w <= remaining);
        match chosen {
            None => break,
            Some(w) => {
                p.cells.push(Cell {
                    name: SmolStr::from(format!("decap_{ri}_{k}")),
                    width: w,
                    height: row.height,
                    position: Point::new(x, row.origin.y),
                    fixed: true,
                });
                *inserted += 1;
                *k += 1;
                x += w;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::*;

    fn empty_placement() -> Placement {
        let mut p = Placement::new();
        p.add_row(Row {
            origin: Point::new(0, 0),
            site_width: 1,
            num_sites: 100,
            height: 10,
        });
        p
    }

    #[test]
    fn insert_taps_at_pitch() {
        let mut p = empty_placement();
        let n = insert_taps(&mut p, 2, 20, TapKind::Both);
        // Expected ~5 taps (every 20, width 2, row span 100).
        assert!((4..=6).contains(&n), "got {n} taps");
    }

    #[test]
    fn insert_decaps_fills_remaining_gap() {
        let mut p = empty_placement();
        // Place a single 30-wide signal cell at x=50.
        p.add_cell(Cell {
            name: "sig".into(),
            width: 30,
            height: 10,
            position: Point::new(50, 0),
            fixed: false,
        });
        let n = insert_decaps(&mut p, &[8, 4, 2, 1]);
        assert!(n > 0);
        // Total decap width should fill ~70 DBU (100 - 30 = 70).
        let decap_total: i64 = p
            .cells
            .iter()
            .filter(|c| c.name.starts_with("decap_"))
            .map(|c| c.width)
            .sum();
        assert_eq!(decap_total, 70);
    }

    #[test]
    fn taps_dont_collide_with_existing_cells() {
        let mut p = empty_placement();
        // Pre-place a wide cell exactly where a tap would go.
        p.add_cell(Cell {
            name: "big".into(),
            width: 5,
            height: 10,
            position: Point::new(0, 0),
            fixed: false,
        });
        insert_taps(&mut p, 2, 20, TapKind::Both);
        // No tap should overlap (0..5).
        for c in p.cells.iter().filter(|c| c.name.starts_with("tap_")) {
            assert!(
                c.position.x >= 5 || c.position.x + c.width <= 0,
                "tap at x={} overlaps signal cell",
                c.position.x
            );
        }
    }

    #[test]
    fn empty_decap_widths_inserts_nothing() {
        let mut p = empty_placement();
        let n = insert_decaps(&mut p, &[]);
        assert_eq!(n, 0);
    }
}
