# Fuzzing — methodology, run results, and regression catalog

The workspace ships five `cargo-fuzz` (libfuzzer-based) harnesses
under [`fuzz/`](fuzz/), one per hand-rolled parser. Each harness
upholds a single contract:

> The parser MUST terminate without panicking on any byte sequence.
> Malformed input returns a typed error; valid input returns `Ok(...)`.

A panic, abort, hang, OOM, or address-sanitizer hit on any input is
a bug. When the harness finds one, the artifact is moved to
`klayout-io/tests/regression/` and a corresponding `#[test]` is
added to [`klayout-io/tests/regressions.rs`](klayout-io/tests/regressions.rs)
so the bug stays fixed.

This document captures the methodology and the catalog of bugs the
fuzz pass has surfaced to date.

## Why this matters

EDA tools regularly ingest multi-GB binaries from semi-trusted
sources: foundry PDK kits, customer GDS handoffs, IP-vendor Liberty
files. A parser that panics on a malformed record is at minimum a
denial of service; in worst cases (memory corruption, OOB writes),
it's a remote-code-execution surface. Fuzz coverage is the only
systematic way to surface these bugs before they bite a user.

## Targets

| Target | Format | Entry point |
|--------|--------|-------------|
| `fuzz_gds` | GDSII binary stream | `klayout_io::read_gds_bytes` |
| `fuzz_oasis` | OASIS binary stream (incl. CBLOCK / modal variables) | `klayout_io::read_oasis_bytes` |
| `fuzz_lef` | LEF (text) | `klayout_lef::read_lef` |
| `fuzz_def` | DEF (text) | `klayout_lef::read_def` |
| `fuzz_liberty` | Liberty `.lib` (text) | `klayout_liberty::parse_liberty` |

## Running

```bash
cargo install cargo-fuzz                                     # one-time
cd fuzz
cargo +nightly fuzz run fuzz_gds -- -max_total_time=300       # 5 min
cargo +nightly fuzz run fuzz_oasis -- -max_total_time=300 -rss_limit_mb=2048
```

Replay an existing crash artifact:

```bash
cargo +nightly fuzz run fuzz_oasis artifacts/fuzz_oasis/crash-<hash>
```

## Most recent run — all targets clean

| Target | Runs | Coverage | Features | Crashes |
|--------|------|----------|----------|---------|
| `fuzz_gds` | 4.78 M / 60 s | high | n/a | 0 |
| `fuzz_oasis` | 210 K / 180 s | 2,860 | 8,000 | 0 |
| `fuzz_lef` | 4.29 M / 120 s | 343 | 1,539 | 0 |
| `fuzz_def` | 3.12 M / 120 s | 658 | 1,848 | 0 |
| `fuzz_liberty` | 53.1 M / 120 s | 146 | 445 | 0 |

(`exec/s` varies between targets — `fuzz_liberty` runs at ~440 K/s
because Liberty is a permissive text grammar that bails on bad
input quickly; `fuzz_oasis` runs at ~1 K/s because OASIS exercises
multi-step decode + interpolation.)

## Bug catalog — 14 bugs found and fixed

All sites were live for an entire fuzz session before being patched
in this document's parent commit. Every fix replaces panicking
arithmetic / allocation with checked arithmetic + a typed `IoError`
return.

### OASIS (13 bugs)

The OASIS reader's complexity (modal variables, repetition records,
multiple point-list encodings, CBLOCK compression) made it the
fuzz target with the most surface area.

| # | Hash | Class | Location (file:line) | Fix |
|---|------|-------|----------------------|-----|
| 1 | `crash-ba2557b3` | `i64` mul overflow | `oasis/read.rs:1323` (g-delta × grid) | `checked_mul` + `IoError` |
| 2 | `crash-8914dff5` | unsigned underflow | `oasis/read.rs:1159` (manhattan polygon `count − 1` when `count == 0`) | early reject |
| 3 | `crash-91cee62d` / `oom-4fc8a8d1` | OOM (multi-TB alloc) | `oasis/read.rs` (point-list `Vec::with_capacity(count)` from input-derived count) | `MAX_POINT_LIST_COUNT = 2²²` cap |
| 4 | `crash-14116bd9` | `Vec::resize` capacity overflow | `oasis/read.rs:147,159` (CELLNAME / TEXTSTRING refnum tables) | `MAX_REF_TABLE_SIZE = 2²²` cap |
| 5 | `crash-acd8c8de` | `i64` add overflow | `oasis/read.rs:1316` (x-arbitrary point list, kind 4/5) | count cap + `checked_mul`/`checked_add` |
| 6 | `crash-f3b44eb4` | overflow + unbounded iter | `oasis/read.rs:1367,1373` (diag matrix kind 9, arbitrary g-delta kind 10/11) | count cap + `saturating_*` |
| 7 | `crash-5e8403e3` | OOM | `oasis/read.rs:1291–1305` (`decode_repetition` kinds 1/2/3 from input `nx, ny`) | `cap_count` helper |
| 8 | `oom-c6186ee6` | OOM | `oasis/read.rs:1369` (`decode_repetition` kind 8 — matrix general) | `cap_count` |
| 9 | `oom-e1c8c407` | OOM (product) | `oasis/read.rs` (`nx · ny` blew past `usize` even when individuals were capped) | new `cap_product` helper bounds the product |
| 10 | `crash-9480efcc` | `i64` add overflow | `oasis/read.rs:199` (RECTANGLE `x + dx + w` accumulation under repetition) | per-term `checked_add`, skip offending entry |
| 11 | `crash-469cd739` | `i64` add overflow | `oasis/read.rs:748` (POLYGON hull vertex accumulation) | `checked_add` + `IoError` |
| 12 | `crash-7dca93b5` | `i64` add overflow | `oasis/read.rs:1551` (CIRCLE facet `cx + dx`) | `saturating_add` |
| 13 | `crash-69f77422` | `i64` add overflow | `oasis/read.rs:1218` (manhattan-polygon delta `fold` accumulator) | `saturating_add` loop |

### GDS (1 bug)

| # | Hash | Class | Location | Fix |
|---|------|-------|----------|-----|
| 14 | `crash-bed320ce` | OOB read on truncated record | `gds/read.rs:296` (STRANS record indexed `data[0..2]` without checking `data.len() ≥ 2`) | length guard |

## OOM hardening

The OOM-class artifacts deserve a separate note because they
revealed a different operational characteristic than the
panicking-arithmetic bugs.

### Single-input bounds

All saved OASIS reproducers — every artifact previously catalogued
as `crash-*` or `oom-*` — now run **in 1–3 ms each under a 256 MB
per-call malloc cap**. The hard sanity bounds (`MAX_POINT_LIST_COUNT`
= `MAX_REF_TABLE_SIZE` = 2²² ≈ 4 M; the `cap_product` guard on
repetition `nx · ny`) make it impossible for a single malformed file
to drive a single allocation into the multi-GB regime.

The parser also now tracks two new aggregate budgets, checked at
each insertion site:

| Budget | Cap | Checked at |
|--------|-----|------------|
| Total cells per file | `MAX_CELLS = 2²⁰` (~1 M) | every `CELL` / `CELL_REF` record |
| Total shapes per file | `MAX_SHAPES = 2²⁶` (~64 M) | every `RECTANGLE / POLYGON / PATH / TEXT / TRAPEZOID / CTRAPEZOID / CIRCLE` push |

A hostile input that repeats `RECTANGLE` records 100 M times now
bails with a typed `IoError` after 64 M, instead of growing the
process toward the kernel's OOM-killer threshold. Both numbers are
~100× any sane chip layout.

### Multi-iteration RSS in libfuzzer

A second class of `oom-*` artifacts surfaced under tight per-process
RSS limits (`-rss_limit_mb=512`). On inspection, these are **not**
single-input allocation bombs:

```bash
$ cargo +nightly fuzz run fuzz_oasis oom-ae2dd0bb -- -rss_limit_mb=256
Executed oom-ae2dd0bb in 242 ms       # well under cap
```

What's happening: libfuzzer reuses the same process across millions
of iterations. The system allocator (jemalloc / system malloc)
holds onto freed pages for reuse rather than returning them to the
OS, and `DashMap` retains its segment buffers. Across millions of
inputs that each allocate 10–100 MB transiently, the process RSS
climbs toward the cap even though no single iteration leaks.

This is a libfuzzer-process-level constraint, not a parser bug. At
the realistic per-process RSS (2 GB), `fuzz_oasis` sustains
**343,751 runs in 241 seconds with zero OOMs**.

For CI fuzzing the recommended invocation is:

```bash
# Realistic per-process budget; new findings are real bugs.
cargo +nightly fuzz run fuzz_oasis -- -max_total_time=3600 -rss_limit_mb=2048

# Aggressive per-iteration assertion; some false positives expected.
cargo +nightly fuzz run fuzz_oasis -- -max_total_time=600 -rss_limit_mb=512
```

### Wall-time-asserted regression tests

Each saved OOM artifact is locked in by [`klayout-io/tests/regressions.rs`](klayout-io/tests/regressions.rs)
under an `assert_quick` helper that asserts rejection in
`< 500 ms`. Without the caps in place, the parser would either OOM
or loop for many seconds over a 4 M-element vector; the wall-time
budget catches the latter long before the former. Current actual
times across all 5 OOM regressions: **2 ms each.**

```rust
const OOM_REJECT_BUDGET: Duration = Duration::from_millis(500);

fn assert_quick<F: FnOnce()>(f: F) {
    let start = Instant::now();
    f();
    assert!(start.elapsed() < OOM_REJECT_BUDGET, ...);
}
```

## Bug class taxonomy

The pattern across all 14 bugs falls into two recurring classes:

1. **Untrusted size driving allocation.** OASIS records carry
   variable-length integers that decode to arbitrary `u64`. A
   malicious file can put `2⁶³` in a count field, and the parser
   used to pass that straight to `Vec::with_capacity` or
   `Vec::resize`. The fix is a hard cap (`MAX_POINT_LIST_COUNT` =
   `MAX_REF_TABLE_SIZE` = 2²² ≈ 4 M) and a typed `IoError` return
   well above any plausible legitimate value but well below
   `usize::MAX`.

2. **`i64` arithmetic on input-derived values.** The OASIS reader
   accumulates polygon vertices, repetition offsets, and rectangle
   corners as `i64` sums. Under Rust's debug overflow checks (and
   under AddressSanitizer), summing extreme deltas panics. The fix
   is `checked_add` / `checked_mul` (returning `IoError`) at the
   accumulation sites where surfacing the overflow matters, or
   `saturating_*` for derived geometry where clamping is
   semantically harmless.

A bulk-clippy sweep (`clippy::arithmetic_side_effects`) would have
caught most of these statically — that's a worthwhile follow-up.

## Regression-test pattern

Every reproducer becomes a permanent test, parallel-named to its
crash artifact and stored under `klayout-io/tests/regression/`:

```rust
// klayout-io/tests/regressions.rs
#[test]
fn oasis_g_delta_grid_overflow_does_not_panic() {
    let bytes = include_bytes!("regression/oasis_g_delta_grid_overflow.oas");
    // Contract: no panic. Either Ok(...) or Err(IoError::...) is fine.
    let _ = read_oasis_bytes(bytes);
}
```

The contract is "no panic"; the parser is free to return `Ok` (if
the malformed record happens to be in a section that's tolerantly
skipped) or `Err` (if the validation check fires). Either is
acceptable. **What's not acceptable, and what the test catches, is a
regression that re-introduces the panic.**

## Coverage measurement

```bash
cargo +nightly fuzz coverage fuzz_gds
cargo cov -- show \
  -instr-profile=fuzz/coverage/fuzz_gds/coverage.profdata \
  target/<host-triple>/coverage/<host-triple>/release/fuzz_gds \
  -format=html -output-dir=fuzz/coverage/fuzz_gds/html
```

Aim for ≥ 80 % line coverage of each parser module. Holes typically
mean the seed corpus doesn't exercise that grammar branch. Seeds
under `fuzz/corpus/<target>/` can be expanded by copying valid files
from `validation/corpus/` (which is what jump-started OASIS coverage
from 27 → 2,860 points after seeding).

## Future work

- **`clippy::arithmetic_side_effects` sweep** to surface the
  remaining `i64` arithmetic sites without waiting for fuzz to
  find each one.
- **`cargo-fuzz coverage` baseline checked in.** Would let CI fail
  on coverage regressions, not just on crash regressions.
- **Differential fuzzing**: feed the same input to klayout-rs and
  to KLayout's C++ engine; assert the same accept/reject decision.
  Catches a third class of bug — accept-where-reject and vice versa
  — that single-side fuzzing misses.
- **Sustained fuzz budget in CI.** Each target run nightly for ~1 h
  with artifacts uploaded; new findings auto-create issues. We
  built the harness, this would close the operational loop.

## Adding a new fuzz target

1. Add a new `.rs` file under `fuzz/fuzz_targets/`. Mirror the
   shape of `fuzz_gds.rs` — `#![no_main]` + `fuzz_target!` calling
   the parser entry point.
2. Register a `[[bin]]` for it in `fuzz/Cargo.toml`.
3. Seed `fuzz/corpus/<target>/` with as many valid example files
   as the workspace produces (typically `validation/corpus/<format>/`).
4. Run `cargo +nightly fuzz run <target> -- -max_total_time=600` and
   commit any reproducer that surfaces.
5. Add a row to the catalog above when the bug is fixed.
