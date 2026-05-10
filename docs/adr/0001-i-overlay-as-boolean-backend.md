# ADR-0001 — `i_overlay` as the boolean-ops backend

**Status:** Accepted

## Context

Boolean operations on polygons (union, intersection, difference, XOR,
symmetric-difference) are the most-used primitive in the workspace —
DRC, PEX, geometry transformations, validation, and the writers all
ultimately reduce to a small set of boolean ops on `Region`. Three
candidate backends were considered:

1. **Hand-rolled Bentley–Ottmann sweep.** Direct control over
   numerical edge cases, integer coordinates, and degenerate-input
   handling. Estimated ~3000 LOC for production-grade quality plus a
   long tail of corner-case bugs surfaced through the validation
   suite.
2. **Wrap CGAL via `cxx`.** The reference implementation in CAD;
   battle-tested. C++ build dependency is unacceptable for our
   "Rust-only, builds on stock toolchain" goal, and CGAL's licensing
   (GPL-3 for production use) doesn't compose with our license
   intent.
3. **Use [`i_overlay`](https://crates.io/crates/i_overlay).** Pure
   Rust, integer-coordinate, MIT-licensed, actively maintained,
   passes a substantial proptest battery against parity oracles in
   its own test suite.

## Decision

Adopt `i_overlay` as the workspace's boolean-ops backend.

## Consequences

**Pro:**
- Zero C/C++ build dependency.
- Integer-exact arithmetic; no float epsilon to tune.
- The validation suite's 30 boolean-ops cases pass byte-exactly
  against `klayout.db`'s C++ engine.

**Con:**
- We're tied to `i_overlay`'s API stability. A breaking change in a
  point release is a workspace-wide refactor.
- Long-narrow polygon slivers at extreme coordinates show ±1 DBU
  bbox drift through `intersection` (documented in
  `klayout-geom/tests/extreme_coords.rs`). Acceptable for a v1; a
  field-solver-grade workflow would need closer audit of the
  precision regime.
- We don't easily extend the backend to L-shapes / arbitrary curve
  segments; arc-aware boolean ops would require either a different
  backend or a polyline approximation pass.

## Revisit if

- We need arbitrary-curve boolean ops for analog or RF flows.
- `i_overlay` upstream stalls or its license changes.
- Sliver-imprecision regressions surface in the validation corpus.
