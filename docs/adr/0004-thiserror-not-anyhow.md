# ADR-0004 — `thiserror` typed errors at the library boundary

**Status:** Accepted

## Context

Every klayout-rs crate publishes `Result<T, Error>` from its public
fns. The error type can be:

1. **`anyhow::Error`** — type-erased, fast to add new error variants,
   ergonomic at the top level. Loses information at `?`-propagation
   boundaries; downstream code can't match on specific errors
   without `downcast_ref`.
2. **`thiserror`-derived enum** per crate, transparently `From`-able
   between crates that need to nest one another's errors. Verbose
   to define, exact to consume.
3. **`Box<dyn Error>`** — same shape as `anyhow` minus the trait
   features. No reason to use this in modern Rust.

## Decision

Library-level errors use `thiserror`-derived enums. `IoError`,
`LefError`, `LibertyError`, `StaError`, `SdfError`, etc. each list
their own variants with explicit `(file: usize, line: usize)` or
similar diagnostic context. Cross-crate nesting uses `#[from]`.

`anyhow` is declared in `[workspace.dependencies]` for downstream
consumers building CLI binaries on top of klayout-rs (where
type-erased errors are appropriate at the very top). It is **not**
used inside any klayout-rs library crate.

## Consequences

**Pro:**
- Downstream code can match on `LefError::UnknownMacro(name)` and
  recover gracefully — common in batch flows that keep going past
  first error.
- Error types document the parser's failure modes in one place.
- `#[error("expected `{expected}` at line {line}, got `{got}`")]`
  produces actionable messages without per-call-site formatting.

**Con:**
- Adding a new failure mode is a 3-line addition (variant + Display
  attribute + `From` if cross-crate) instead of a 1-line `bail!()`.
- Some sites have to wrap a foreign-crate error type that doesn't
  implement `From`; the result is slightly more `.map_err(|e| ...)?`
  noise than `anyhow` would have.

## Revisit if

- Top-level binary crates start consuming klayout-rs and the
  erased-error ergonomics become a bottleneck — at which point
  the binary crate adopts `anyhow` while the libraries stay typed.
