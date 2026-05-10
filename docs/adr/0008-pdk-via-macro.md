# ADR-0008 — PDK declarations via the `pdk!` macro, not data files

**Status:** Accepted

## Context

A "PDK" is the set of layer mappings, port-kind tags, design rules,
and DBU convention a flow targets. Two ways to consume one:

1. **Runtime config (TOML / YAML / JSON).** Familiar; non-Rust
   contributors can edit. Loses compile-time checking — typos in
   layer names become runtime errors.
2. **Compile-time macro that generates a typed struct.** `klayout-pdk`
   ships `pdk! { layers: { m1 => (1, 0), ... } }`. Each PDK becomes
   a strongly-typed handle; the rest of the workspace consumes it
   via `MyPdk::M1` which fails at compile time on typos.

## Decision

The `pdk!` macro from `klayout-pdk`. Each consumer's PDK lives in
its own crate (or its own module), which keeps proprietary PDK data
out of the shared workspace. We do **not** check in any production
PDK — those are NDA'd. Synthetic toy PDKs in tests live alongside
the test code, not as workspace fixtures.

## Consequences

**Pro:**
- Compile-time errors on layer-name typos.
- Distinct PDK consumers can coexist in one binary without runtime
  collision.
- Zero dependency on a config-parsing library.
- Per-port-kind data can be associated as constants.

**Con:**
- Adding a new PDK is a "write Rust" operation, not a "edit YAML"
  one.
- Macro hygiene: declarative macros are simpler to read but harder
  to extend than a procedural macro would be. We've not yet hit
  that ceiling.
- Tools that want to ingest PDKs at runtime (e.g. a Python
  binding) need a translator step.

## Revisit if

- Large numbers of contributors start writing PDKs and the Rust
  build-cycle becomes a barrier (procedural macro could read a
  YAML file at compile time, getting the best of both).
