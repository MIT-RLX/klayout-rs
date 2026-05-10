# klayout-rs fuzz harnesses

`cargo-fuzz` (libfuzzer-based) coverage of the workspace's hand-rolled
parsers. Each target asserts the same contract:

> The parser MUST terminate without panicking on any byte sequence.
> Malformed input returns a typed error; valid input returns `Ok(...)`.

A panic, abort, hang, or address-sanitizer hit on any input is a bug.

## Why this matters

EDA tools regularly ingest multi-GB binaries from semi-trusted sources:
foundry PDK kits, customer GDSII handoffs, IP-vendor libraries. A
parser that panics on a malformed record is at minimum a denial of
service; in worst cases (memory corruption, OOB writes), it's a
remote code execution surface. Fuzz coverage is the only systematic
way to surface these bugs before they bite a user.

## Targets

| Target | Format | Entry point |
|--------|--------|-------------|
| `fuzz_gds` | GDSII binary stream | `klayout_io::read_gds_bytes` |
| `fuzz_oasis` | OASIS binary stream (incl. CBLOCK / modal variables) | `klayout_io::read_oasis_bytes` |
| `fuzz_lef` | LEF (text) | `klayout_lef::read_lef` |
| `fuzz_def` | DEF (text) | `klayout_lef::read_def` |
| `fuzz_liberty` | Liberty `.lib` (text) | `klayout_liberty::parse_liberty` |

## Running

```bash
# One-time install (nightly toolchain required for libfuzzer-sys).
cargo install cargo-fuzz

# Run a target until you stop it (Ctrl-C).
cd fuzz
cargo +nightly fuzz run fuzz_gds

# Run with a time bound (90 minutes typical for a CI scheduled job).
cargo +nightly fuzz run fuzz_oasis -- -max_total_time=5400

# Replay a crash artifact after a finding.
cargo +nightly fuzz run fuzz_lef artifacts/fuzz_lef/crash-deadbeef
```

## Corpus seeds

`corpus/<target>/` holds seed inputs. Bootstrap each target's corpus
with samples that exercise as much of the grammar as possible:

```bash
# GDS — feed the validation suite's GDS test files as seeds.
cp ../validation/corpus/gds/*.gds corpus/fuzz_gds/

# OASIS — same.
cp ../validation/corpus/oasis/*.oas corpus/fuzz_oasis/

# LEF / DEF / Liberty — pull from a real PDK.
# (Don't check in PDK files — they're typically confidential.)
```

## CI integration

A scheduled (e.g. nightly) CI job should run each target for ~15 min
and fail if any new artifacts appear under `fuzz/artifacts/<target>/`.
Findings should be reduced to a minimum reproducer with
`cargo +nightly fuzz tmin` and committed as a regression test in the
parser's `tests/` directory.

## Coverage measurement

```bash
cargo +nightly fuzz coverage fuzz_gds
cargo cov -- show \
  -instr-profile=fuzz/coverage/fuzz_gds/coverage.profdata \
  target/x86_64-unknown-linux-gnu/coverage/x86_64-unknown-linux-gnu/release/fuzz_gds \
  -format=html -output-dir=fuzz/coverage/fuzz_gds/html
```

Aim for ≥ 80% line coverage of the parser modules. Holes typically
mean the seed corpus doesn't exercise that grammar branch.
