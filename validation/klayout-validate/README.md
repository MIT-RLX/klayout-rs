# klayout-validate

Differential-validation oracle for the
[`klayout-rs`](https://github.com/MIT-RLX/klayout-rs) workspace.

Compares klayout-rs results against KLayout's reference C++ engine
(`klayout.db`) across ~3000 corpus cases covering `db.Trans`,
`db.Box`, `db.Region`, `db.Layout` (GDSII + OASIS read/write), and
assorted DRC checks.

## How it works

1. A Python script (`oracle.py`, in the parent `validation/`
   directory) drives KLayout's reference engine through a fixed-seed
   PRNG.
2. The corpus is serialized to JSON and checked in.
3. This crate's tests load the corpus and assert byte-exact equality
   with klayout-rs.
4. `parity_report` prints a Markdown summary; current state is
   **3296 / 3296 cases pass** across all suites with an oracle.

CI does **not** need `klayout.db` installed — the corpus is the
contract. Regenerate only when adding cases or upgrading the
reference KLayout.

## Run

```bash
cargo test -p klayout-validate --test parity_report -- --nocapture
```

This crate has `publish = false`; it ships with the workspace but is
never released to crates.io.

## License

Licensed under GPL-3.0-only.
