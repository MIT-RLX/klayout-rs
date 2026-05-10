# Examples

Runnable demos showing the workspace composing into working flows.

## `end_to_end.rs`

Walks a 16-cell shift-register-like design through every back-end stage
the workspace currently implements: quadratic global placement →
Tetris legalization → DME clock-tree synthesis → RSMT topology
extraction → Pathfinder global routing → DRC width check → STA
arrival/required propagation with worst-slack reporting → GDSII write.

```bash
cargo run -p klayout --example end_to_end --release
```

Expected output (numbers will drift as algorithms improve):

```text
=== klayout-rs end-to-end demo ===
[1] netlist built: 18 cells, 17 nets, HPWL = 9050
[2] quadratic placement: HPWL 9050 → 7929 (-12.4%)
[3] legalize: HPWL → 7950 (delta +21)
[4] CTS DME: 8 buffers, skew 0 DBU, total wire 8075 DBU
[5] RSMT for the 8 FF positions: 0 Steiner points, total length 3610 DBU
[6] Pathfinder: 9 routes in 1 iterations, residual overflow 0
[7] geometry written into top cell
[7b] DRC width(min=50) on M2: 0 violation polygons
[8] STA: worst-slack node = ff0/Q (-3.4600 ns)
[9] GDS written to /tmp/klayout-rs-demo/demo_top.gds
```

The negative slack at the end is intentional — the demo's clock period
is intentionally tight relative to the placeholder cell-arc delays so
the STA stage produces a non-trivial worst-slack endpoint to look at.

### What this demo proves

- Every API in the workspace composes through one driver program.
- `Library` survives being built up incrementally across stages.
- The crate boundaries don't leak — a single `use klayout::*` import
  reaches every back-end algorithm.
- The output GDSII is byte-stable and round-trippable through the
  reader.

### What this demo does NOT prove

- Algorithm quality on real designs — see `BENCHMARKS.md` for that.
- Sign-off accuracy — the timing numbers use placeholder fixed
  cell-arc delays, not Liberty-driven NLDM lookups, so the slack is
  illustrative.
- Multi-corner / multi-mode flow — single-corner demonstration only.

### Inspecting the result

Open `/tmp/klayout-rs-demo/demo_top.gds` in any GDS viewer (KLayout,
Magic, GDSEditor) to see the placed cells and routed wires. Layer
mapping:

| Layer | Number | Purpose |
|-------|--------|---------|
| M1 | 1/0 | Cell footprints |
| M2 | 2/0 | Routed wires + clock-tree branches |
