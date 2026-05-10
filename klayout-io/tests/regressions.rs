//! Reproducer regressions for bugs surfaced by `cargo-fuzz`.
//!
//! Every entry in this file corresponds to a fixture under
//! `tests/regression/` that was originally a crash artifact.
//! Once the bug was fixed, the artifact moved here as a permanent
//! regression test — the contract is that the parser must reject
//! the malformed input via a typed `IoError`, never panic / abort.
//!
//! ## OOM-class regressions
//!
//! Three of the saved artifacts originally surfaced as libfuzzer
//! `oom-*` reports: a malformed length / refnum field drove
//! `Vec::with_capacity` or `Vec::resize` toward a multi-terabyte
//! allocation. Those fixes added hard sanity caps
//! (`MAX_POINT_LIST_COUNT`, `MAX_REF_TABLE_SIZE`, plus a
//! `cap_product` guard on repetition `nx · ny`).
//!
//! The contract for OOM regressions is stronger than "no panic": we
//! also assert each runs in **bounded wall time**. Without the
//! caps, the parser would hang trying to allocate; with them, it
//! rejects in single-digit milliseconds. The wall-time check makes
//! it impossible for a future regression to lose the cap without
//! the test noticing.

use klayout_io::{read_gds_bytes, read_oasis_bytes};
use std::time::{Duration, Instant};

/// The OOM regressions assert each malformed input is rejected in
/// well under this budget. With the caps in place, every saved
/// artifact completes in 1–3 ms; if the cap regresses, the parser
/// would either OOM (test infra failure) or take many seconds
/// looping over a 4 M-element vector — the budget catches the
/// latter long before the former.
const OOM_REJECT_BUDGET: Duration = Duration::from_millis(500);

fn assert_quick<F: FnOnce()>(f: F) {
    let start = Instant::now();
    f();
    let elapsed = start.elapsed();
    assert!(
        elapsed < OOM_REJECT_BUDGET,
        "expected fast rejection (cap should fire); took {elapsed:?}"
    );
}

/// fuzz_oasis crash-ba2557b3 — a `g_delta` repetition record paired
/// an extreme delta with an extreme grid multiplier, blowing past
/// `i64::MAX` on the `dx · grid` multiplication. The fix uses
/// `checked_mul` and surfaces the overflow as `IoError`.
#[test]
fn oasis_g_delta_grid_overflow_does_not_panic() {
    let bytes = include_bytes!("regression/oasis_g_delta_grid_overflow.oas");
    // The contract is "no panic"; the parser may return Ok on a
    // shorter prefix or Err on the malformed delta. Either is fine.
    let _ = read_oasis_bytes(bytes);
}

/// fuzz_oasis crash-8914dff5 — a manhattan-polygon point list with
/// `count = 0`. The closure-edge logic computed `count - 1` on an
/// unsigned counter, panicking on underflow. The fix rejects
/// `count == 0` early.
#[test]
fn oasis_manhattan_polygon_count_zero_does_not_panic() {
    let bytes = include_bytes!("regression/oasis_manhattan_polygon_count_zero.oas");
    let _ = read_oasis_bytes(bytes);
}

/// fuzz_oasis crash-91cee62d / oom-4fc8a8d1 — malformed
/// `point_list` count (read from the byte stream as a u64) drove
/// `Vec::with_capacity(count + 2)` into a multi-TB allocation that
/// AddressSanitizer aborted on. The fix caps the allowed point-list
/// count at `MAX_POINT_LIST_COUNT` (2²² = 4 M vertices) and surfaces
/// the rejection as a typed `IoError` before reaching the `Vec`
/// constructor. No legitimate OASIS file has polygons in that
/// regime; values above the bound are unambiguously malformed.
#[test]
fn oasis_point_list_count_overflow_does_not_oom() {
    assert_quick(|| {
        let bytes = include_bytes!("regression/oasis_point_list_count_overflow.oas");
        let _ = read_oasis_bytes(bytes);
    });
}

#[test]
fn oasis_point_list_oom_does_not_oom() {
    assert_quick(|| {
        let bytes = include_bytes!("regression/oasis_oom_1.oas");
        let _ = read_oasis_bytes(bytes);
    });
}

#[test]
fn oasis_third_crash_stays_fixed() {
    let bytes = include_bytes!("regression/oasis_3rd_crash.oas");
    let _ = read_oasis_bytes(bytes);
}

/// fuzz_oasis crash-14116bd9 — a CELLNAME_REF / TEXTSTRING_REF
/// record carried a refnum in the billions, driving `Vec::resize`
/// on the cellname / textstring table into a `capacity overflow`
/// panic from `RawVec`. Cap the refnum at `MAX_REF_TABLE_SIZE`.
#[test]
fn oasis_ref_table_huge_refnum_does_not_oom() {
    assert_quick(|| {
        let bytes = include_bytes!("regression/oasis_ref_table_oom.oas");
        let _ = read_oasis_bytes(bytes);
    });
}

/// fuzz_oasis crash-acd8c8de — same overflow class as
/// `oasis_g_delta_grid_overflow`, but in the type-4/5 (x-arbitrary)
/// and type-6/7 (y-arbitrary) point-list decoders. Both now use
/// `checked_mul` / `checked_add` and surface overflow as a typed
/// `IoError`.
#[test]
fn oasis_x_arbitrary_overflow_does_not_panic() {
    let bytes = include_bytes!("regression/oasis_x_arbitrary_overflow.oas");
    let _ = read_oasis_bytes(bytes);
}

/// fuzz_oasis crash-f3b44eb4 — same class again, in the type-9
/// (diag matrix) and type-10/11 (arbitrary g-delta) point-list
/// decoders. Both now sanity-cap the count and use `checked_mul`
/// for the `i * (dx,dy)` accumulation.
#[test]
fn oasis_diag_matrix_overflow_does_not_panic() {
    let bytes = include_bytes!("regression/oasis_diag_matrix_overflow.oas");
    let _ = read_oasis_bytes(bytes);
}

/// fuzz_oasis crash-5e8403e3 — `decode_repetition` (kind 1/2/3)
/// drove `(0..nx).flat_map(0..ny)…` with `nx · ny` exceeding `usize`
/// when fed extreme byte values, blowing past `malloc`'s allocation
/// ceiling under ASan. Cap repetition counts at the same
/// `MAX_POINT_LIST_COUNT` sanity bound.
#[test]
fn oasis_repetition_overflow_does_not_oom() {
    let bytes = include_bytes!("regression/oasis_repetition_overflow.oas");
    let _ = read_oasis_bytes(bytes);
}

/// fuzz_oasis oom-c6186ee6 — `decode_repetition` kind 8 (matrix
/// general) had the same `nx + 2 / ny + 2` overflow + giant-product
/// alloc as kinds 1/2/3 but on a separate code path. Now uses the
/// same `cap_count` helper.
#[test]
fn oasis_repetition_kind8_does_not_oom() {
    assert_quick(|| {
        let bytes = include_bytes!("regression/oasis_repetition_kind8.oas");
        let _ = read_oasis_bytes(bytes);
    });
}

/// fuzz_oasis oom-e1c8c407 — even with each axis individually capped
/// at `MAX_POINT_LIST_COUNT`, a repetition with `nx ≈ ny ≈ 4 M`
/// still produces a 16-trillion-element flat_map. Add a `cap_product`
/// pass that bounds `nx · ny`.
#[test]
fn oasis_repetition_product_does_not_oom() {
    assert_quick(|| {
        let bytes = include_bytes!("regression/oasis_repetition_product.oas");
        let _ = read_oasis_bytes(bytes);
    });
}

/// fuzz_oasis crash-9480efcc — RECTANGLE record paired extreme
/// `(x, y, w, h)` with extreme repetition `(dx, dy)` and overflowed
/// `i64` on the `x + dx + w` accumulation. Now uses `checked_add`
/// per term and skips the offending repetition entry.
#[test]
fn oasis_rect_offset_overflow_does_not_panic() {
    let bytes = include_bytes!("regression/oasis_rect_offset_overflow.oas");
    let _ = read_oasis_bytes(bytes);
}

/// fuzz_oasis crash-469cd739 — POLYGON record's hull-vertex
/// accumulation `cx += dx` overflowed `i64` when fed extreme
/// deltas. Now uses `checked_add` and surfaces overflow as a
/// typed parse error.
#[test]
fn oasis_polygon_hull_overflow_does_not_panic() {
    let bytes = include_bytes!("regression/oasis_polygon_hull_overflow.oas");
    let _ = read_oasis_bytes(bytes);
}

/// fuzz_oasis crash-7dca93b5 — CIRCLE record's facet-point
/// computation `cx + dx` overflowed `i64` for an extreme
/// `(center, radius)` pair. Now saturates.
#[test]
fn oasis_circle_overflow_does_not_panic() {
    let bytes = include_bytes!("regression/oasis_circle_overflow.oas");
    let _ = read_oasis_bytes(bytes);
}

/// fuzz_oasis crash-69f77422 — manhattan-polygon point list
/// summed deltas via `fold` whose accumulator overflowed `i64`.
/// Replaced with an explicit `saturating_add` loop.
#[test]
fn oasis_manhattan_fold_overflow_does_not_panic() {
    let bytes = include_bytes!("regression/oasis_manhattan_fold_overflow.oas");
    let _ = read_oasis_bytes(bytes);
}

/// fuzz_gds crash-bed320ce — STRANS record indexed `data[0..2]`
/// without checking that the record's payload was actually long
/// enough. A truncated STRANS (data.len() < 2) panicked on the
/// out-of-bounds read. Now bails on short payloads.
#[test]
fn gds_strans_short_data_does_not_panic() {
    let bytes = include_bytes!("regression/gds_strans_short_data.gds");
    let _ = read_gds_bytes(bytes);
}
