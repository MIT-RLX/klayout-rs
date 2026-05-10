# ADR-0009 — `rlx` consumed via `[patch.crates-io]`, not as a workspace member

**Status:** Accepted

## Context

`rlx` is a tensor / ML compiler workspace developed in parallel with
klayout-rs. Some klayout-rs crates may eventually depend on `rlx-*`
sub-crates (e.g. for ML-driven placement / routing experiments).
Three ways to wire it up:

1. **Add `rlx` as a workspace member of klayout-rs.** Forces
   single-repo coupling; klayout-rs builds become slower (compile
   the entire ML stack); release cadences entangle.
2. **Wait until `rlx` ships on crates.io and depend on the
   published version.** The right end-state, but blocks current
   work because rlx isn't published yet.
3. **`[patch.crates-io]` directives** that point `rlx-*` at a
   local checkout (or eventually a git URL). Cargo activates the
   patch only when a klayout-rs crate actually depends on `rlx-*`;
   inert patches print a "not used" warning but don't fail the
   build.

## Decision

Option 3. The workspace `Cargo.toml` carries
`[patch.crates-io]` entries for every published `rlx-*` sub-crate,
pointing at `/Users/Shared/rlx/rlx-{ir,opt,...}`. The Containerfile
strips these entries with `sed` because `/Users/Shared/rlx` doesn't
exist inside the container; in-container builds verify klayout-rs
is self-contained.

## Consequences

**Pro:**
- Current work isn't blocked on rlx publication.
- When a klayout-rs crate adds an `rlx-* = "0.1"` dep, the patch
  silently activates — no other change needed in this repo.
- The Containerfile demonstrates that klayout-rs builds standalone;
  the patch is a development convenience, not a runtime dependency.

**Con:**
- Cargo prints "patch ... was not used" warnings until a klayout-rs
  crate actually adopts rlx — informational only, but noisy.
- The patch file path is absolute; contributors with rlx checked
  out elsewhere need to override.
- Once rlx publishes 0.1.0 to crates.io, this patch block must
  be removed (or repointed at a git revision tag).

## Revisit if

- rlx publishes to crates.io. At that point the patch block is
  removed and klayout-rs picks up the published version directly.
- The path-based patch becomes a portability blocker for
  contributors. At that point switch to a git URL with a fixed
  rev: `rlx-ir = { git = "...", rev = "..." }`.
