# klayout-rs roadmap

Status snapshot of differential-parity coverage and what's left to close it.

## Where we are

**4893 / 4893 cases passing (100.00%)** under strict vertex/value
equality, across 14 suites and three reference oracles:

- `klayout.db` (Python) — trans, bbox, drc, gds, oasis, region,
  polygon_ops, cif, dxf, mag.
- `klayout-rs-oracle:latest` Docker image (extends `openroad/orfs`):
  - OpenSTA's `read_liberty` for liberty.
  - OpenDB's `read_lef` / `read_def` for lef / def.
  - Bundled `spef_dump.py` (independent Python reader) for spef.

The corpus is checked in; CI runs the Rust side without needing Docker
or `klayout.db` installed. `cargo test -p klayout-validate --test
parity_report -- --nocapture` is the regression gate.

## Coverage gaps (corpus-bound)

Our parsers handle more of each format than the corpus exercises. The
following surfaces are read by the parser but not asserted in any
fixture yet:

### DEF

Import tests in `klayout-lef/tests/def_import_coverage.rs` and
`tests/parity.rs` now assert `SPECIALNETS` (stripes), design `VIAS`,
`GROUPS`, `REGIONS`, `BLOCKAGES`, multi-segment routes with vias, and
`*` coordinate wildcards. The OpenDB JSON corpus in
`validation/klayout-validate/tests/def.rs` still targets regular `NETS`
geometry plus placement (special nets are not wire-dumped from OpenDB
the same way). Still thin: `PROPERTYDEFINITIONS`, scan chains, STYLE /
SHIELD on routes, and very large PDK-style special-net RECT mixes.

### LEF

- `SPACING` rules (within-layer, parallel-edge, end-of-line).
- Antenna rules (`ANTENNAGATEAREA`, `ANTENNADIFFAREA`).
- Extra `VIARULE` / generated-via flavours beyond the `VIA` definitions
  covered in `tests/parity.rs`.
- Polygon (non-rect) pin ports in corpus-scale fixtures.

### Liberty
- `timing()` arcs (delay/transition NLDM tables).
- `leakage_power()` per-state groups.
- `internal_power()` rise / fall energy lookups.
- `statetable` for sequential cells.
- AOCV / POCV / CCS table groups.
- Library-level `time_unit` / `voltage_unit` / `capacitance_unit`
  scaling.

### CIF
- `W` (wire) command — width-bearing path.
- `DS` per-cell scale parameter (currently assumes 1:1).
- Comments embedded in cell bodies.

### OASIS
- `TEXT` records (we have a writer fixture but no reader corpus).
- `TRAPEZOID`, `TRAPEZOID_A`, `TRAPEZOID_B`, `CTRAPEZOID` (handled by
  the parser, no corpus).
- `CIRCLE` (parser approximates as 64-vertex polygon, no corpus).
- Property records and standard-property names (S_BOUNDARY etc.).

### GDS
- Multi-layer layouts beyond what the existing 14 fixtures cover.
- PROPVALUE / PROPATTR records.

### MAG
- `use` cell-instance blocks (parser supports them, no corpus).
- Magic `flabel` / `rlabel` text records.

### DXF
- `CIRCLE` and `TEXT` entities (parser supports, no corpus).
- `ARC` and `ELLIPSE`.
- `INSERT` (block instance) — DXF hierarchy.

## Crates with no differential coverage at all

These have unit tests but are not compared against any reference tool:

- `klayout-pdk` — PDK loading, layer mapping, deck integration.
- `klayout-place` — global / quadratic / force-directed placement.
- `klayout-route` — multilayer / detailed / global+detailed routing.
- `klayout-cts` — clock tree synthesis.
- `klayout-sta` — static timing analysis (NLDM / CCS / POCV / AOCV).
- `klayout-spatial` — spatial indexing.
- `klayout-deck` — DRC deck loading.
- `klayout-connect` — hierarchical LVS, hierarchical PEX.
- Manufacturing modules: OPC, density-fill, LFD, pin-optimisation,
  crosstalk analysis.

The reference tool for each is identifiable but not yet wired:

| crate | reference oracle | image / install |
|---|---|---|
| place | OpenROAD `replace` / `dpl` / `gpl` | already in `klayout-rs-oracle` |
| route | OpenROAD `tritonroute` / `fastroute` | already in image |
| cts   | OpenROAD `triton_cts` | already in image |
| sta   | OpenSTA timing reports | already in image |
| pdk   | KLayout `db.LayerMap` | already via `klayout.db` |
| spatial | RTreeStar reference impl | pure-Rust, no oracle needed |
| connect (LVS) | KLayout `db.LayoutToNetlist` | already via `klayout.db` |
| connect (PEX) | OpenSTA SPEF round-trip | already in image |
| OPC / density-fill / LFD | Calibre / Pegasus (proprietary) | no open oracle |

## Realistic-scale fixtures

All current fixtures are tiny (under a few KB). No coverage at:

- **PDK-scale layouts** — sky130 / asap7 / nangate45 standard cells.
- **Real OpenROAD outputs** — gcd / ibex / aes from OpenROAD's
  `OpenROAD-flow-scripts` benchmark suite.
- **Million-shape stress tests** — for boolean ops and DRC.
- **Multi-cell hierarchies > 3 deep**.

## Test-infrastructure gaps

- **Docker image is AMD64-only** — runs under emulation on Apple
  Silicon (~10× slower than native). An `arm64` build would speed up
  oracle regen significantly.
- **No CI hookup for the Docker oracle** — both `oracle.py` and
  `oracle_external.py` rely on a developer running them locally
  before committing corpus changes.
- **No fuzzing / property-based tests** beyond the 1500 / 1548
  cases the trans / bbox suites already have. Adding `proptest` or
  `cargo-fuzz` for the GDS / OASIS readers would catch malformed-input
  bugs the corpus can't reach.
- **Workspace toolchain friction** — the `rlx-*` patch block in the
  workspace `Cargo.toml` needs `edition2024` (Cargo ≥ 1.94), but
  `rust-toolchain.toml` pins 1.83. Tests need
  `RUSTUP_TOOLCHAIN=1.94.0` to build. Either drop the patch block from
  the workspace, or update the toolchain pin.

## Suggested next-round priorities

In rough order of value-per-effort:

1. **DEF SPECIALNETS + VIAS** — unlocks real-PDK DEF ingest.
2. **Liberty `timing()` arcs** — required for any meaningful STA
   parity, and `klayout-sta` is a major surface that has zero
   differential coverage.
3. **klayout-place vs OpenROAD `replace`** — first non-format crate
   with a Docker oracle path. Establishes the pattern for the other
   ASIC-flow crates.
4. **PDK-scale fixtures** — pull a few sky130 cells / a small ibex
   block from `OpenROAD-flow-scripts` and run them through every
   reader. Catches the long tail of edge cases that hand-written
   fixtures miss.
5. **arm64 oracle image** — quality-of-life for Apple-Silicon
   contributors. Either find / build a native OpenROAD arm64 binary
   or split the Docker image so only `read_liberty` / `read_lef` /
   `read_def` paths need OpenROAD (the SPEF and pure-Python paths
   could run native).
6. **CI integration** — at minimum, run `cargo test -p
   klayout-validate` on every PR (the corpus is checked in, no
   Docker / Python needed). Optionally gate corpus regeneration on a
   scheduled job that does have Docker.
7. **`klayout-sta` vs OpenSTA** — once Liberty timing arcs are
   parsed, compare end-to-end timing reports for a small design.

## What "100% parity" means today

For every assertion the corpus contains, klayout-rs produces the same
output as the reference tool — vertex-for-vertex on geometry,
value-for-value on properties. There are no normalization fudges or
"close enough" comparisons left in the test layer.

That's a meaningful but bounded claim. It does not mean klayout-rs
implements every feature of every format. It means: where we have
ported a feature, the port matches the reference; and where we don't
yet have differential coverage, the gap is documented above.
