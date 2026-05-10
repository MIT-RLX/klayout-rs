# ADR-0010 — `deny(warnings)` is the build policy

**Status:** Accepted

## Context

Rust compiler and clippy lints are warnings by default. Three
policies:

1. **Default (`warn`).** Warnings accumulate over time; nobody
   notices. New contributors push code that emits warnings; the
   build still succeeds; cleanup is a recurring chore.
2. **`#![deny(warnings)]` per crate.** Surfaces immediately, but
   maintainers fight the compiler whenever a future rustc release
   tightens an existing lint — every minor toolchain bump becomes
   a workspace fix-up sprint.
3. **`[workspace.lints.rust] warnings = "deny"`** with an explicit
   pinned `rust-toolchain.toml`. The pin is what makes (3) work
   without (2)'s churn — we promote new compiler warnings on our
   schedule, not the toolchain's.

## Decision

Option 3.

- `[workspace.lints.rust] warnings = "deny"` denies all rustc
  warnings.
- `[workspace.lints.clippy]` denies a small, opinionated set
  (`todo`, `unimplemented`, `dbg_macro`); clippy is allowed to
  *warn* on the rest, which becomes deny via the rust gate.
- Each crate opts in via `[lints] workspace = true`.
- `rust-toolchain.toml` pins to `1.94.0` so a stricter `1.95`
  doesn't silently break the build.

We deliberately skip `clippy::unwrap_used` / `clippy::expect_used`
— every checked-in unwrap/expect was reviewed in the audit pass and
either eliminated or documented with an invariant; those lints
would false-positive every audited site.

## Consequences

**Pro:**
- Zero accumulated warnings, ever. The repo enforces what
  reviewers used to enforce by hand.
- New contributors see compiler errors instead of warnings → they
  fix the issue at submit time, not "later".
- The pinned toolchain limits when new lints become breaking — we
  audit one toolchain bump at a time, not every nightly.

**Con:**
- Every minor toolchain bump is a "go through and fix the new
  lint" PR.
- Clippy's `unwrap_used` / `expect_used` are off, which means new
  unwraps land via code review, not via the lint. Acceptable
  tradeoff because the lint is too noisy on test code.
- A new contributor with a `rustup default nightly` setup may see
  more denied warnings than the pinned toolchain shows.

## Revisit if

- Toolchain bumps become disproportionately painful (likely in
  the rare case where a major rustc tightening breaks dozens of
  call sites at once).
- We adopt `clippy::pedantic` — that's a much bigger denied set
  and we're not there.
