# ADR-0005 — Differential validation via checked-in JSON corpus

**Status:** Accepted

## Context

Geometric / parser correctness in EDA is famously hard to verify in
isolation: there are too many corner cases (degenerate polygons,
coincident edges, rounding-tied coordinates, etc.) for unit tests to
cover, and the "correct" answer is whatever the reference tool does,
not what a textbook says.

Three options for the validation contract:

1. **Hand-write expected-value unit tests.** Direct, easy to
   debug. Doesn't scale past a few dozen cases; encodes the test
   author's interpretation rather than ground truth.
2. **Live differential testing.** Run klayout-rs and the reference
   tool side-by-side on every test invocation, compare. Requires
   the reference (KLayout's `klayout.db`, OpenSTA, OpenDB) to be
   installed on every CI runner — heavyweight, slow, brittle.
3. **Pre-recorded differential corpus.** Run the reference tool
   once (with a fixed-seed PRNG over inputs) and serialize results
   as JSON to a checked-in `corpus/` directory. Tests load JSON
   and compare. CI doesn't need the reference installed.

## Decision

Option 3. The corpus generators (`validation/oracle.py` for
KLayout C++, `validation/oracle_external.py` for OpenSTA / OpenDB
via Docker) are run by maintainers when adding cases or upgrading
the reference; the JSON corpus is checked in; the Rust-side tests
load and assert byte-equality.

Currently 3310 cases across 11 sub-suites pass with this contract.

## Consequences

**Pro:**
- CI runners need only a Rust toolchain — no Python, no Docker, no
  KLayout install. Builds in 30 seconds.
- The corpus is auditable: a contributor can `git diff
  validation/corpus/region.json` to see exactly what changed
  between reference versions.
- Single source of truth — the corpus *is* the contract. If the
  reference tool's output changes, regenerating the corpus is the
  intentional act of accepting the new behavior.
- Multi-oracle: we run against KLayout C++ for geometry/I/O and
  Docker-OpenSTA for Liberty. Each suite documents its oracle.

**Con:**
- The corpus encodes the reference's bugs as ground truth. We
  partially mitigate by running multi-oracle (no single tool's
  bugs become canonical) and by documenting known divergences in
  module-level docstrings.
- ~few hundred MB of checked-in JSON over time.
- Regenerating the corpus is a maintainer step that requires
  installing the reference tool.

## Revisit if

- Corpus size becomes a repo-bloat concern (compress with `zstd`
  per-suite).
- A reference tool's behavior diverges in a way that breaks
  multiple downstream tests at once — at that point we may need a
  versioned-corpus strategy.

See [`validation/ORACLES.md`](../../validation/ORACLES.md) for the
contract on adding a new oracle.
