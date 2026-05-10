# ADR-0006 — No `unsafe` in the workspace

**Status:** Accepted

## Context

Rust's `unsafe` keyword unlocks raw pointer dereferences, FFI, mutable
statics, and a few other escape hatches. An EDA tool processes
multi-GB binaries from semi-trusted sources (foundry PDKs, customer
GDS handoffs); a memory-safety bug here is a CVE waiting to happen.

Three policies:

1. **Allow `unsafe` freely** — typical for systems crates that
   transitively call into FFI. Fast in micro-benchmarks; risky on
   the parser surface.
2. **`unsafe` only with explicit justification** in module docs.
   The standard for most production Rust.
3. **`unsafe` forbidden in this workspace.** We rely on safe
   abstractions for every hot path; if we need `unsafe` semantics,
   we pull a vetted crate from `crates.io` that already wears the
   safety review (e.g. `parking_lot` for fast mutexes, `dashmap`
   for concurrent hashmap, `i_overlay` for boolean ops).

## Decision

`#![forbid(unsafe_code)]` is implicit in the workspace policy. We
have not added the lint annotation per crate yet — pending — but
`grep -r unsafe klayout-*/src` returns zero matches today.

## Consequences

**Pro:**
- Memory-safety review reduces to "did you write `unsafe`?" — the
  answer being `no` simplifies the audit dramatically.
- Fuzz harnesses have stronger guarantees (no UB possible from
  klayout-rs code itself; any UB would be in a dep or in libfuzzer
  instrumentation).
- The rust-only "no C/C++ build" promise extends to the safety
  story.

**Con:**
- We give up the absolute fastest paths in a few places (e.g.
  manual SIMD on the boolean-ops hot loop).
- We're locked into the abstraction surface our deps provide; if
  we ever need to talk to a C library directly (say, a CCS-spec
  parser written in C++), we'd revisit.

## Revisit if

- A profiler shows >10% of wall-time spent in a hot loop where a
  hand-rolled `unsafe` SIMD path would clear the bottleneck.
- We adopt a foreign C/C++ dependency (rlx-cuda, klayout.db) and
  need an FFI shim. At that point this ADR becomes "no `unsafe` in
  algorithm code; FFI shims allowed in `*-sys`-style sub-crates
  only."
