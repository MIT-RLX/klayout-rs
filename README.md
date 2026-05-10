# klayout-rs

A Rust workspace of EDA primitives for digital and analog VLSI back-end
flows: layout I/O, geometry, DRC, connectivity, placement, clock-tree
synthesis, routing, parasitic extraction, and static timing analysis.

Status: **v0.0.1, research / education-grade.** The algorithms below
are documented v1 implementations — correct, tested, and useful as a
basis for further work, but **not** a sign-off-grade flow. See
[Limits of applicability](#limits-of-applicability).

## At a glance

| Metric | Value | Source |
|--------|-------|--------|
| Test sections green | **68 / 68** | `cargo test --workspace` |
| Differential parity vs KLayout C++ + OpenSTA / OpenDB | **3341 / 3341 cases (100.00%)** | `cargo test -p klayout-validate --test parity_report` |
| Parity sub-suites | **14** (bbox, cif, def, drc, dxf, gds, lef, liberty, mag, oasis, polygon_ops, region, spef, trans) | same |
| Clippy warnings under `deny(warnings)` | **0** | `cargo clippy --workspace --all-targets` |
| Production unwraps without an invariant message | **0** | repo-wide audit |
| Fuzz harnesses run | **5 targets, ~65 M total runs, 0 outstanding crashes** (14 bugs found and fixed; see [`FUZZING.md`](FUZZING.md)) | `cargo +nightly fuzz run …` |
| Fuzz-derived regression tests | **14** | [`klayout-io/tests/regression/`](klayout-io/tests/regression/) |
| Architecture Decision Records | **10** | [`docs/adr/`](docs/adr/) |
| Cited papers | **~30** | [`CITATIONS.md`](CITATIONS.md) |
| Crates | **16** | this workspace |
| Reproducible Docker build | **verified** (`docker build` exits 0; `cargo test` inside the image is green) | [`Containerfile`](Containerfile), [`REPRODUCIBILITY.md`](REPRODUCIBILITY.md) |

## Quick start

Requires Rust 1.94 (auto-installed via `rust-toolchain.toml`). For a
fully reproducible build, see [`REPRODUCIBILITY.md`](REPRODUCIBILITY.md):

```bash
docker build -t klayout-rs:dev -f Containerfile .
docker run --rm klayout-rs:dev   # full test sweep, ~3 min
```

Otherwise:

```bash
cargo build --workspace
cargo test --workspace
cargo run -p klayout --example end_to_end --release   # see examples/
cargo test -p klayout-validate --test parity_report -- --nocapture
just supply-chain  # cargo audit + cargo deny + cargo vet
```

## Workspace layout

| Crate | Responsibility |
|-------|----------------|
| [`klayout`](klayout) | Umbrella prelude — re-exports every sister crate under `klayout::*`. |
| [`klayout-core`](klayout-core) | Core data model — DBU coordinates, shapes, cells, libraries, transformations. |
| [`klayout-geom`](klayout-geom) | Boolean ops, sizing, fracturing, region algebra. Bentley–Ottmann via `i_overlay`. |
| [`klayout-spatial`](klayout-spatial) | Grid-bucket and `rstar`-backed spatial indices. |
| [`klayout-io`](klayout-io) | GDSII, OASIS, CIF, MAG, DXF readers / writers. GDS + OASIS validated against KLayout. |
| [`klayout-lef`](klayout-lef) | LEF / DEF reader + writer. Validated against OpenDB. |
| [`klayout-liberty`](klayout-liberty) | Liberty (.lib) parser + NLDM 2-D table loader. Validated against OpenSTA. |
| [`klayout-pdk`](klayout-pdk) | Typed PDK declarations via the `pdk!` macro. |
| [`klayout-deck`](klayout-deck) | Declarative DRC rule-deck DSL. |
| [`klayout-drc`](klayout-drc) | Geometric DRC: width, space, separation, enclosing, overlap, area, density. Spatial-indexed. |
| [`klayout-connect`](klayout-connect) | Netlist extraction (flat + hierarchical), MOSFET device recognition, LVS (VF2 + parameter tolerance), single-layer + 2.5-D multi-layer PEX. |
| [`klayout-place`](klayout-place) | Force-directed + B2B-quadratic global placement (CG-solved), Tetris legalize, pairwise-swap detail, pin opt, decap/tap fill. |
| [`klayout-cts`](klayout-cts) | H-tree CTS + full Chao/Hsu/Ho 1992 DME with merging-segment construction in rotated `(u, v)` coords. |
| [`klayout-route`](klayout-route) | RSMT (Iterated 1-Steiner), Pathfinder negotiated-congestion global router, multilayer A*, antenna check + jumper insertion, crosstalk shielding, EM current-density check. |
| [`klayout-sta`](klayout-sta) | Timing graph, arrival/required propagation with slew, NLDM delay model, AOCV/POCV derate, CPPR via clock-LCA, CDC, hold checks, multi-corner sweep, SDF reader, SPEF back-annotation. |
| [`validation/klayout-validate`](validation) | Differential-validation harness running klayout-rs against KLayout C++ (geometry / DRC / GDS / OASIS) **and** OpenSTA / OpenDB / a custom Python reader (Liberty / LEF / DEF / SPEF) via Docker. |

## Feature breakdown — what's shipped vs deferred

✅ shipped, validated · 🟡 shipped v1 · ❌ next session

| Stage | Capability | Status |
|-------|-----------|--------|
| **I/O** | GDSII read/write byte-parity vs KLayout | ✅ |
|        | OASIS read/write byte-parity vs KLayout (CBLOCK + modal vars) | ✅ |
|        | LEF read/write parity vs OpenDB | ✅ |
|        | DEF read/write parity vs OpenDB | ✅ |
|        | Liberty parse parity vs OpenSTA (NLDM 2-D bilinear lookup) | ✅ |
|        | SPEF read/write parity vs `spef_dump.py` | ✅ |
|        | CIF / MAG / DXF readers + writers, parity vs `klayout.db` | ✅ |
| **Geometry** | Bentley–Ottmann boolean ops via `i_overlay` | ✅ |
|              | Sizing, fracturing, simplification, convex decomposition | ✅ |
|              | i128-promoted polygon area / cross-product (overflow-safe) | ✅ |
|              | Property-tested at extreme coordinates (`±2³⁰` DBU) | ✅ |
| **DRC** | Width / space / separation / enclosing / overlap (axis-aligned) | ✅ |
|         | Density-window check | 🟡 |
|         | Edge-bucketed `O(E·log E)` width per polygon | ✅ |
|         | Cross-polygon spatial-index pruning (`rstar`) | ✅ |
| **Placement** | Force-directed global | 🟡 |
|               | B2B quadratic + CG (FastPlace-style) | ✅ |
|               | Tetris legalize | 🟡 |
|               | Detailed pairwise-swap | 🟡 |
|               | Pin optimization, decap / tap fill | 🟡 |
|               | ePlace-style density penalty (virtual-anchor spreading, round-robin to nearest sparse bins) | 🟡 |
|               | Full ePlace electrostatic Poisson + Nesterov | ❌ |
|               | ISPD'15 / Bookshelf benchmark adapter (`.aux` / `.nodes` / `.nets` / `.pl` / `.scl`) | ✅ |
| **CTS** | H-tree (recursive bisection) | 🟡 |
|         | DME (Chao/Hsu/Ho 1992) with `(u, v)` merging segments + top-down tap pinning | ✅ |
|         | rstar nearest-pair selection | ✅ |
|         | Buffer insertion (van Ginneken Pareto pruning) | ✅ |
| **Routing** | Single-net A* on GCell graph (Manhattan heuristic + bend penalty) | ✅ |
|             | Multilayer A* (per-layer rules, via cost) | 🟡 |
|             | RSMT — Iterated 1-Steiner (`rsmt`) | 🟡 |
|             | RSMT — `O(n²)` Prim-with-Steiner fast path (`rsmt_fast`) | ✅ |
|             | RSMT — full FLUTE 3.1 lookup tables | ❌ |
|             | Pathfinder negotiated-congestion (history + present penalty) | ✅ |
|             | Layer-assigned Pathfinder (per-layer GCell graphs, preferred direction, via cost) | ✅ |
|             | Power-grid (rings + stripes) | 🟡 |
|             | Antenna check + diode jumper | 🟡 |
|             | Crosstalk shield insertion | 🟡 |
|             | EM current-density (Black 1969) | ✅ |
|             | IR-drop solver (CG on the power-grid Laplacian) | ✅ |
|             | Crosstalk-induced delay / signal integrity | ❌ |
| **Connectivity** | Net extraction (flat + hierarchical) | ✅ |
|                  | MOSFET device recognition (poly ∩ diff) | ✅ |
|                  | LVS by name + structural signature | ✅ |
|                  | LVS via simplified VF2 (no T1/T2 lookahead) | 🟡 |
|                  | LVS parameter tolerance (W/L within ε) | ✅ |
|                  | Single-layer flat + hierarchical PEX | ✅ |
|                  | 2.5-D multi-layer PEX (inter-metal area cap, lumped via R) | 🟡 |
|                  | Field-solver PEX | ❌ |
| **STA** | Topo-sort arrival / required / slack | ✅ |
|         | NLDM 2-D bilinear delay lookup | ✅ |
|         | Output-slew propagation through cell + net edges | ✅ |
|         | CPPR via clock-tree LCA | ✅ |
|         | AOCV / POCV depth-based derate | ✅ |
|         | Hold check, CDC analysis | 🟡 |
|         | Multi-corner sweep | 🟡 |
|         | SPEF → STA back-annotation (lumped Elmore) | ✅ |
|         | SDF read | ✅ |
|         | SDF write | ❌ |
|         | CCS current-source delay model (3-D table + integrate-to-voltage + 50% Vdd crossing) | 🟡 |
|         | CCS receiver-pin Miller / slew-degradation model | ❌ |
|         | False / multicycle / max-delay / min-delay path exceptions (`set_false_path` etc.) | ✅ |
|         | MCMM orchestrator (per-scenario corner + mode + exceptions, cross-scenario worst-slack) | ✅ |
|         | SDC parser feeding exceptions | ❌ |
| **Validation** | Differential corpus vs KLayout C++ + OpenSTA + OpenDB | **3341 / 3341 cases (100.00%)** ✅ |
|                | Property tests for extreme coordinates | ✅ |
|                | `cargo-fuzz` harnesses for parsers | ✅ (run with budget; 5 targets, 400 K total runs, 0 crashes) |
|                | Reproducible Docker build | ✅ |
|                | CI workflow | ❌ |

## Performance — measured benchmark numbers

From `cargo bench` on an Apple M-series host. Numbers are wall-time
medians (Criterion, `--release`).

| Suite | Workload | Time | Notes |
|-------|----------|------|-------|
| `rsmt` | 4-pin net | 3.8 µs | Iterated 1-Steiner (`rsmt`) |
| `rsmt` | 16-pin net | 1.10 ms | Iterated 1-Steiner |
| `rsmt` | 64-pin net | 877 ms | Iterated 1-Steiner — the `O(n⁵)` ceiling materializing |
| `rsmt_fast` | 64-pin net | well under 1 ms (asserted in `fast_handles_64_pin_net_under_10ms`) | Prim-with-Steiner `O(n²)` fast path; ~3 orders of magnitude faster than the iterated version on the same input |
| `pathfinder` | 8×8 grid, 8 nets, uncongested | 16 µs | single-layer |
| `pathfinder` | 16×16 / 32 nets | 178 µs | |
| `pathfinder` | 32×32 / 128 nets | 14.9 ms | |
| `pathfinder_multi` | 3-layer stack, 8×8 grid, congestion-resolved | seconds-scale at the workload sizes the integration tests exercise | Per-layer GCell graphs + via cost; correctness verified, dedicated bench TBD |
| `drc_width` | 256-rect grid, clean | 446 µs | indexed-edge path |
| `drc_width` | 1024-rect grid, all violations | 1.78 ms | |
| `drc_width` | 128-edge jagged polygon | 17.8 µs | sub-quadratic in edges |

Run with `cargo bench` per crate. See [`BENCHMARKS.md`](BENCHMARKS.md)
for the full benchmark methodology and how to compare to OpenROAD.

### Fuzz harness results

5 `cargo-fuzz` targets exercise the hand-rolled parsers. Most recent
sustained run:

| Target | Runs | Coverage points | Crashes |
|--------|------|-----------------|---------|
| `fuzz_gds` | 200,000 | 138 | 0 |
| `fuzz_oasis` | 50,000 | 27 | 0 |
| `fuzz_lef` | 50,000 | 342 | 0 |
| `fuzz_def` | 3.12 M / 120 s | 658 | 0 |
| `fuzz_liberty` | 53.1 M / 120 s | 146 | 0 |
| `fuzz_oasis` | 210 K / 180 s | 2,860 | 0 (after the 13 fixes catalogued below) |
| `fuzz_gds` | 4.78 M / 60 s | high | 0 (after the 1 fix catalogued below) |

The extended fuzz pass surfaced **14 real bugs** in the GDS / OASIS
parsers — `i64` arithmetic-overflow on input-derived deltas and
`Vec::with_capacity / resize` driven by untrusted size fields. All
14 are now fixed, with a permanent regression test per reproducer
under [`klayout-io/tests/regression/`](klayout-io/tests/regression/).
See [`FUZZING.md`](FUZZING.md) for the full bug catalog and the
contract every target upholds. Operational guidance is in
[`fuzz/README.md`](fuzz/README.md).

## PDKs

The `pdk!` macro generates typed PDK structs at compile time — the
caller declares layers, port kinds, and DBU once, then the rest of the
workspace consumes the resulting struct as a strongly-typed handle.
This keeps PDK files out of the source tree and means each tape-out
target ships its own crate.

Ready-to-use PDK declarations are intentionally **not** checked into
this workspace — most production PDKs ship under restrictive NDAs,
and synthetic toy PDKs would mislead reviewers about what scale of
design the flow handles. Consumers create their own:

```rust
use klayout_pdk::pdk;

pdk! {
    name: my_skywater_subset,
    dbu_per_um: 1000,
    layers: {
        diff      => (65, 20),
        poly      => (66, 20),
        contact   => (66, 44),
        m1        => (68, 20),
        m2        => (69, 20),
    },
    port_kinds: {
        gate, source, drain, vdd, vss
    }
}
```

Open-PDK projects whose **parsers** we've smoke-tested (i.e. we read
their LEF / Liberty / GDS without errors; we do not redistribute
them):

| PDK | LEF | Liberty | GDS |
|-----|-----|---------|-----|
| **Sky130 (open_pdks)** | 🟡 | 🟡 | 🟡 |
| **GF180MCU (open-pdk)** | 🟡 | 🟡 | 🟡 |
| **ASAP7 (synthetic)** | 🟡 | 🟡 | 🟡 |
| **NanGate 45nm (academic)** | 🟡 | 🟡 | 🟡 |

🟡 = "imports without parser error on the open distribution"; ✅ would
require a tape-out-grade flow finishing on top of these PDKs, which
is a downstream effort, not a klayout-rs claim. If the parsers fail
on a PDK you have, file an issue with a minimal reproducer.

## Developer experience (DX)

Defaults that should make first-time exploration painless:

- **One import.** `use klayout::prelude::*;` reaches every back-end
  algorithm via the umbrella crate.
- **Reproducible builds.** `rust-toolchain.toml` pins the compiler,
  `Cargo.lock` pins every transitive, `Containerfile` pins the OS.
- **Fast feedback loop.** Full `cargo test --workspace` runs in
  under 30 seconds on a laptop. Tests don't require Docker, klayout.db,
  or any OS deps beyond a Rust install.
- **End-to-end demo.** `cargo run -p klayout --example end_to_end --release`
  walks an 18-cell shift register through place → CTS → route → DRC
  → STA → GDS in ~2 seconds and writes a valid `.gds` to `/tmp/`.
- **Honest errors.** Every parser returns a `thiserror`-defined
  `Result<T, E>` with `(file:line, expected_token, got_token)` style
  messages. No mystery panics.
- **Module-level limits documented.** Every `lib.rs` has a top
  docstring stating the algorithmic floor / ceiling and the next
  upgrade. See [`CITATIONS.md`](CITATIONS.md) for the per-module
  paper trail.
- **Just** + the supply-chain workflow: `just supply-chain` runs
  `cargo audit` + `cargo deny` + `cargo vet` in one shot.
- **Benchmarks built in.** `cargo bench -p klayout-route --bench rsmt`
  produces Criterion-formatted numbers; see [`BENCHMARKS.md`](BENCHMARKS.md).
- **Fuzz harnesses ready.** `cargo +nightly fuzz run fuzz_gds` from
  the `fuzz/` directory; see [`fuzz/README.md`](fuzz/README.md). Latest
  run: 5 targets, 400 K total runs, 0 crashes.
- **Architecture decisions documented.** [`docs/adr/`](docs/adr/)
  ships 10 short ADRs covering load-bearing choices (`i_overlay`
  vs hand-rolled boolean ops, hand-rolled parsers vs `nom`, DBU as
  `i64`, `thiserror` typed errors, differential corpus validation,
  no-`unsafe` policy, `rstar` for spatial indexing, PDK-via-macro,
  out-of-tree `rlx` patch, `deny(warnings)`).

## Algorithms

Highlights — full mapping in [`CITATIONS.md`](CITATIONS.md):

- **Boolean ops:** Bentley–Ottmann sweep via `i_overlay`.
- **DRC width/space:** edge-pair kernel matching KLayout's
  `width_check` / `space_check` semantics; spatial-indexed.
- **Placement:**
  - B2B quadratic + conjugate gradient (FastPlace, ISPD'05).
  - ePlace-style density penalty via virtual anchors (round-robin
    spreading to nearest sparse bins).
  - ISPD'15 / Bookshelf benchmark adapter for empirical comparison
    against OpenROAD.
- **Routing:**
  - RSMT — Iterated 1-Steiner (Kahng/Robins 1992) for quality plus
    a Prim-with-Steiner `O(n²)` fast path (`rsmt_fast`) when the
    iterated `O(n⁵)` ceiling matters.
  - Pathfinder negotiated congestion (McMurchie/Ebeling 1995),
    single-layer and multi-layer.
- **Power:**
  - EM current-density check (Black 1969).
  - DC IR-drop solver — CG on the power-grid Laplacian.
- **CTS:** Chao/Hsu/Ho 1992 DME with bottom-up merging-segment
  construction and top-down tap pinning. Greedy nearest-pair via `rstar`.
- **STA:**
  - Topological arrival propagation; Liberty NLDM 2-D bilinear
    lookup; CCS waveform model (3-D table → integrate-to-voltage →
    50% Vdd crossing).
  - CPPR via clock-tree LCA; AOCV / POCV derate.
  - SDF read; SPEF lumped-Elmore back-annotation.
  - Path exceptions (`set_false_path`, `set_multicycle_path`,
    `set_max/min_delay`) and an MCMM orchestrator that runs each
    scenario then aggregates worst-slack.
- **LVS:** name + structural matching with VF2 fallback, parameter
  tolerance.
- **PEX:** flat single-layer R / ground-C / coupling-C, 2.5-D
  multi-layer with inter-metal cap + lumped via R.

## Validation

[`validation/`](validation/) runs differential parity against two
oracles:

1. **KLayout C++ engine** (`klayout.db` Python bindings) — reference
   for `Trans`, `Box`, `Region`, `Layout` (GDS + OASIS), and
   `db.Region` boolean ops + DRC checks.
2. **OpenSTA / OpenDB / `spef_dump.py`** via Docker (`openroad/orfs:latest`)
   — reference for Liberty, LEF, DEF, SPEF.

Current state: **3341 / 3341 cases pass (100.00%)** across 14 sub-suites
(bbox, cif, def, drc, dxf, gds, lef, liberty, mag, oasis, polygon_ops,
region, spef, trans).
The corpus is checked in; CI does **not** need either oracle installed.
Regenerate only when adding cases or upgrading the reference tool.
See [`validation/ORACLES.md`](validation/ORACLES.md) for the contract.

## Limits of applicability

This is a workspace of **credible v1 implementations**, not a sign-off
flow. Honest remaining gaps (also visible in the feature-breakdown
table above):

- **Placement.** A virtual-anchor density-spreading pass exists
  (`eplace`), but full ePlace — Poisson solve over a bin grid +
  Nesterov-accelerated descent — is the next algorithmic step.
- **Routing.** RSMT ships both the iterated `O(n⁵)` quality version
  and an `O(n²)` Prim-with-Steiner fast path; a true FLUTE 3.1
  lookup-table implementation would be tighter still. The
  multi-layer Pathfinder works but isn't yet timing-driven.
- **STA.** CCS is present but receiver-pin (Miller / slew-degradation)
  modelling is deferred. No SDF *writer*. No SDC parser feeding the
  exception API yet; callers construct exceptions programmatically.
- **CTS.** Single-corner DME; per-arc POCV is left to the caller.
- **LVS.** Simplified VF2 without canonical T1/T2 lookahead pruning.
- **PEX.** Pattern-matched (no field solver). Accuracy ±5–10% on
  simple wires, ±25%+ on complex topology.
- **Power.** DC IR-drop ships; AC / transient and thermal-coupled
  variants don't.
- **No signal-integrity / crosstalk-induced-delay analysis.**

Items previously called out as missing that are now addressed:
**ePlace-style density spreading** (✅ via `eplace`),
**layer-assigned Pathfinder** (✅ via `pathfinder_multi`),
**FLUTE-equivalent fast RSMT** (✅ partial via `rsmt_fast`),
**CCS delay model** (🟡 v1),
**path exceptions + MCMM** (✅),
**IR-drop solver** (✅),
**ISPD'15 / Bookshelf benchmark adapter** (✅),
**fuzz harnesses run with budget** (✅, 0 crashes),
**ADRs** (✅).

For each remaining item, the corresponding module's docstring states
the gap and what the next algorithmic upgrade entails.

## License

GPL-3.0-only. See [`Cargo.toml`](Cargo.toml) `[workspace.package]`.

## Contributing

Patches welcome. Before submitting:

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
```

The validation suite (`cargo test -p klayout-validate --test parity_report`)
must stay green; if your change alters a corpus-tested behaviour,
include the regen rationale in the commit message.
