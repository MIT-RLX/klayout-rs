# Benchmarks

Criterion-driven micro-benchmarks for the load-bearing algorithms in
each crate. Run with:

```bash
# All benchmarks in a crate.
cargo bench -p klayout-route
cargo bench -p klayout-place
cargo bench -p klayout-drc

# Filter to a single bench function.
cargo bench -p klayout-route --bench rsmt

# Quick smoke run (skip statistics, just verify nothing regressed).
cargo bench -p klayout-route -- --quick

# Compare against a baseline saved earlier.
cargo bench -p klayout-route -- --save-baseline main
git checkout my-feature
cargo bench -p klayout-route -- --baseline main
```

## Targets

| Crate | Bench | What it measures |
|-------|-------|------------------|
| `klayout-route` | `rsmt` | Iterated 1-Steiner wall time vs pin count (4, 16, 64, 256). Throughput is reported in pins/sec. |
| `klayout-route` | `pathfinder` | Pathfinder global-router wall time on synthetic congested workloads (8×8 grid / 8 nets, 16×16 / 32 nets, 32×32 / 128 nets). |
| `klayout-place` | `quadratic` | B2B-quadratic placer wall time vs cell count (64, 256, 1024). |
| `klayout-drc` | `width` | DRC `width` rule on rectangle grids (clean and all-violations) plus a fractured single-polygon case to exercise the per-polygon edge-bucket index. |

## Calibrating against published numbers

The right way to validate algorithmic claims is to run on the same
inputs the established tools publish numbers for:

- **Placement** — ISPD'15 / ISPD'18 benchmark suites
  ([`https://www.ispd.cc/contests/`](https://www.ispd.cc/contests/)).
  Compare HPWL + wall time against OpenROAD's RePlAce port on
  identical inputs.
- **Global routing** — ISPD'18 / ISPD'19 routing benchmarks. Compare
  overflow + wall time against OpenROAD's FastRoute / CUGR.
- **STA** — TAU contests. Compare endpoint slack distributions
  against OpenSTA on a small RISC-V mini-core.

Adapter scripts to ingest those benchmark formats are not in this
workspace yet — they're a natural follow-on. Until then, the synthetic
benchmarks above are useful for *regression detection* (run the same
`--save-baseline` before/after a commit), not for *absolute claims*.

## What to do with the numbers

1. **Track regressions per PR.** A 2× slowdown on `rsmt/64` is a bug.
   Set up a CI job that runs `cargo bench --save-baseline pr-N` and
   posts the diff against `main`.
2. **Surface algorithmic ceilings.** `rsmt` at n=256 is `O(n⁵)`-bound
   — the bench is what would tell you "FLUTE adapter is overdue".
3. **Don't run benches under sanitizers or in CI on shared runners.**
   Benchmark numbers from a noisy machine are worse than no numbers.
   Use a dedicated bench host with CPU-pinning and `nice -n -10`.
