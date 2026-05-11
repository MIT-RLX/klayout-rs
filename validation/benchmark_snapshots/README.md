# Benchmark snapshots (regression baselines)

## Recording

Requires: Rust toolchain (`rust-toolchain.toml`), Python 3.10+, network for Docker PDK `curl`,
optional Docker for `./validation/docker/run_benchmark.sh all`.

Progress prints one ASCII bar line per finished phase on **stderr**; set `RECORD_BENCHMARK_QUIET=1` to disable.

```bash
python3 validation/scripts/record_benchmark_snapshot.py
```

**Full (default):** runs all Criterion cases for `rsmt` (including n=256), with a long timeout for
that bench. For a shorter run (for example in CI), use `--fast` or `RECORD_BENCHMARK_FAST=1`:
smaller sample windows and `rsmt` filtered to sizes 4, 16, and 64 only.

To **fail** when the KLayout oracle image is missing or the docker phase is skipped, use
`--strict-klayout` or `RECORD_BENCHMARK_STRICT_KLAYOUT=1`. Build the image with
`./validation/docker/run_benchmark.sh build-klayout-image`.

This writes:

- `snapshot_<UTC>.json` — structured summary (parity counts, cargo test tally, Docker smoke,
  Criterion medians for the four Rust benches).
- `latest.json` — copy of the newest snapshot (intended baseline for comparisons).
- `raw/*.txt` — full logs for forensic diff when JSON is not enough (not tracked in git; see `raw/.gitignore`).

Criterion sampling is **full** by default (longer `--sample-size` / warm-up / measurement windows).
Fast mode uses shorter windows plus a narrower `rsmt` filter; numbers remain comparable within the
same profile, not canonical publishable timings.

## Comparing later

After recording a candidate snapshot:

```bash
python3 validation/scripts/compare_benchmark_snapshot.py \
  validation/benchmark_snapshots/latest.json \
  validation/benchmark_snapshots/snapshot_<new>.json
```

Exit status `1` if parity cases shrink, pass rate drops, or fewer tests pass than the baseline.

**Notice:** gcd synthesis ODB sha prefix often changes when the ORFS/OpenROAD image changes; the
comparison script reports that separately from hard failures.

## CI

Snapshots are deterministic only for a pinned Rust toolchain + Docker image tag. Bump
`latest.json` intentionally when corpus or toolchain upgrades are intentional.
