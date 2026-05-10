# klayout-connect

Connectivity extraction, device recognition, LVS, and parasitic
extraction for the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace.

## Capabilities

| Module | Function |
|--------|----------|
| `extract` | Flat connectivity extraction → `Netlist`. |
| `hier` / `hier_netlist` | Hierarchical extraction preserving cell boundaries. |
| `device` | MOS device recognition from poly + diff layers. |
| `netlist` | `Netlist` IR — `Net`, `Device`, `Pin`. |
| `lvs` | Flat layout-vs-schematic with name + structural matching. |
| `lvs_hier` | Hierarchical LVS over `HierNetlist`. |
| `vf2` | VF2 graph isomorphism fallback for ambiguous matches. |
| `pex` | Flat single-layer parasitic extraction (R + ground-C). |
| `pex_hier` | Hierarchical PEX. |
| `pex_multilayer` | 2.5-D multi-layer R + via resistance + inter-metal coupling. |
| `pex_skeleton` | Skeleton extraction (no values). |
| `spef` / `spef_hier` | SPEF writer (flat + hierarchical). |
| `cdl` | CDL netlist read for the LVS reference side. |
| `antenna` | Per-net metal-area accumulation + antenna-rule check. |

## How extraction works

Given a `Library`, a `CellId`, and the conductor / label layers,
`extract_flat` produces a `Netlist`:

1. Aggregate shapes touching one another (boolean merge per layer +
   inter-layer via stitching).
2. Each merged piece is one electrically-connected `Net`.
3. Text labels falling inside a piece name the net.

For LVS, see `lvs::compare`. The matcher tries name equality first,
then structural matching, then VF2 with parameter-tolerance comparison.

## Example

```rust,ignore
use klayout_connect::{extract_flat, lvs};

let netlist = extract_flat(&lib, top_cell, &layers)?;
let report  = lvs::compare(&schematic, &netlist, &lvs::Tolerance::default());
assert!(report.matched());
```

## Limits

Simplified VF2 (no canonical T1/T2 lookahead pruning) — works where
connectivity uniquely classifies devices. PEX is pattern-matched, not
field-solved: ±5–10% on simple wires, ±25%+ on complex topology.

## License

Licensed under GPL-3.0-only.
