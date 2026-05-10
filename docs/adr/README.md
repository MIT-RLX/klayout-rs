# Architecture Decision Records

Short documents capturing the load-bearing design decisions in
klayout-rs — what was chosen, what was rejected, and why. Future
contributors read these before re-litigating settled questions.

Format: each ADR is a single page with status (Accepted / Superseded /
Deprecated), context, decision, and consequences. Numbered in the
order they were committed; rejected/changed ADRs are *amended in
place* with a `Superseded by 0042` link.

## Index

| # | Title | Status |
|---|-------|--------|
| [0001](0001-i-overlay-as-boolean-backend.md) | `i_overlay` as the boolean-ops backend | Accepted |
| [0002](0002-handrolled-parsers.md) | Hand-rolled parsers (no `nom` / `pest`) | Accepted |
| [0003](0003-dbu-as-i64.md) | DBU coordinates as `i64`, not `i32` | Accepted |
| [0004](0004-thiserror-not-anyhow.md) | `thiserror` typed errors at the library boundary | Accepted |
| [0005](0005-validation-via-differential-corpus.md) | Differential validation via checked-in JSON corpus | Accepted |
| [0006](0006-no-unsafe-default.md) | No `unsafe` in the workspace | Accepted |
| [0007](0007-rstar-for-spatial-index.md) | `rstar` for the production spatial index | Accepted |
| [0008](0008-pdk-via-macro.md) | PDK declarations via the `pdk!` macro, not data files | Accepted |
| [0009](0009-rlx-as-out-of-tree-patch.md) | `rlx` consumed via `[patch.crates-io]`, not as a workspace member | Accepted |
| [0010](0010-deny-warnings-as-build-policy.md) | `deny(warnings)` is the build policy | Accepted |
