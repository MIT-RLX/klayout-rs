# Validation oracles

This workspace differentially validates against multiple reference
implementations. Each oracle answers the question "what does the
ground-truth tool do on this input?" and the Rust side asserts byte
equality.

## Why multiple oracles

A single-reference oracle encodes the reference's bugs as ground
truth. KLayout's C++ engine is a high-quality reference for layout
geometry, GDSII / OASIS, and DRC. But it isn't authoritative for:

- Liberty / SDF parsing — those are Synopsys's spec and OpenSTA is
  the more widely used parser.
- LEF / DEF parsing — Cadence's spec; OpenROAD's `OpenDB` is the
  canonical open-source consumer.
- SPEF parsing — IEEE 1481; OpenSTA again.
- Routing / placement quality metrics — OpenROAD's RePlAce / FastRoute.

So we run multiple oracles and require every klayout-rs claim to be
testable against at least one.

## Current oracles

| Oracle | Source | Suite directories | Regenerate |
|--------|--------|---------------------|------------|
| **KLayout C++** | `klayout.db` Python bindings | `corpus/{trans,bbox,region,polygon_ops,drc,drc_density_grid,gds/,oasis/}` | `python validation/oracle.py [<suite>]` (use `drc_density_grid` for the combinatorial density suite) |
| **KLayout LEF/DEF layout** | `klayout.db` LEFDEF reader | `corpus/klayout_lefdef/` | `python validation/oracle_klayout_lefdef.py` |
| **OpenROAD CTS + OpenDB** | Docker OpenRO (`validation/oracle_external.py`) | `corpus/cts_downstream/` | `python3 validation/oracle_external.py cts_downstream` (`klayout-rs-oracle:latest` image) |
| **Golden CTS DME metrics** | `klayout-cts` deterministic dump (`dump_cts_dme_corpus` example) | `corpus/cts_dme.json` | `cargo run -q -p klayout-cts --example dump_cts_dme_corpus > validation/corpus/cts_dme.json` |

## Suite anatomy

Each suite ships:

1. **An oracle script** that produces JSON output from the reference
   tool. Determinism is mandatory — fixed PRNG seeds, sorted
   iteration, no embedded timestamps.
2. **JSON corpus** under `corpus/<suite>/` (or `corpus/<suite>.json`
   for monolithic suites). Checked in. CI does not need the oracle
   tool installed; the corpus is the contract.
3. **A Rust integration test** under
   `validation/klayout-validate/tests/<suite>.rs` that loads the JSON
   and asserts byte equality with klayout-rs's output. The test
   fails if klayout-rs ever diverges from the reference.

## Adding a new oracle

Pick a tool the workspace doesn't yet validate against — e.g.:

- **Magic** for an alternative layout-geometry reference.
- **OpenROAD's RePlAce** for placement quality numbers.
- **Yosys** for an alternative netlist source.

### Step 1 — write the oracle script

Generate JSON output that's straight-line decodable. For a tool
running via Docker:

```python
# validation/oracle_<tool>.py
def docker_run(host_dir, script):
    # Mount host_dir at /work, run the tool, capture stdout.
    ...

def regen_<suite>():
    inputs = [...]
    out = {}
    for fixture in inputs:
        result = docker_run(CORPUS, f"<tool> < {fixture}")
        out[fixture] = canonicalize(result)
    (CORPUS / f"{suite}.json").write_text(json.dumps(out, indent=2, sort_keys=True))
```

Shape rules:

- Numeric outputs: keep them as JSON numbers, not strings. Don't
  truncate floats — write enough digits for round-trip.
- Lists of records: sort canonically (e.g. by name) before dumping
  so the corpus diff is meaningful when the tool's iteration
  changes.
- **Document the canonicalization choices in a header comment.**
  Future you will thank present you.

### Step 2 — wire the Rust test

```rust
// validation/klayout-validate/tests/<suite>.rs
use klayout_validate::corpus_path;
use serde_json::Value;

#[test]
fn matches_<tool>_on_<suite>() {
    let path = corpus_path("<suite>.json");
    let expected: Value = serde_json::from_slice(
        &std::fs::read(&path).expect("missing corpus"),
    ).expect("corrupt corpus JSON");
    let got = run_klayout_rs_on(...);
    assert_eq!(got, expected, "<tool> parity drift on <suite>");
}
```

### Step 3 — register in `parity_report.rs`

`tests/parity_report.rs` is the single-page summary that prints all
suites' status. Add an entry to the table and document the
regeneration command.

### Step 4 — add a row to this file

Once the suite is green, append to the "Current oracles" table above.

## What's NOT in this directory

- **Silicon-validated golden files.** A taped-out chip's GDS with
  cross-checked DRC/LVS would be the ultimate validation, but that's
  a different operational scale than CI integration.
- **Performance benchmarks.** See [`BENCHMARKS.md`](../BENCHMARKS.md)
  for those — they answer "is it fast?", not "is it correct?".
- **Fuzz corpora.** [`fuzz/`](../fuzz/) holds the libfuzzer harnesses
  and seeds. Crashes found by fuzzing should be reduced into a
  minimum reproducer and added as a normal regression test.

## Roadmap

Next oracles to add (in order of leverage):

1. **OpenSTA on SPEF** — extend `oracle_external.py spef`. Parse the
   same .spef with both tools, dump per-net `(R, C)` summaries,
   compare. ~100 LOC.
2. **OpenDB on LEF/DEF** — same Docker image. LEF is currently
   validated only against our own writer round-trip; an external
   oracle would catch parser divergence. ~150 LOC.
3. **Magic on layout queries** — `magic` runs locally on most
   Linux/macOS systems without Docker. Useful for a second
   geometry reference independent of i_overlay. ~200 LOC.
4. **OpenROAD RePlAce on placement HPWL** — produces an HPWL number
   per placement; klayout-rs's quadratic placer should land within
   a documented relative tolerance (currently ~30% gap honestly
   reported in the README). ~300 LOC. This is the hardest because
   placement isn't a deterministic transform; the comparison is
   "is the gap to RePlAce within the documented bound on every
   ISPD'15 input?".
