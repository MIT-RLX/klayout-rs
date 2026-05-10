# Reproducible builds

The workspace ships three artifacts that together pin the build
environment so every contributor — and every CI run, and every IEEE
artifact-evaluation reviewer — sees the same compilation outcome:

| File | Pins |
|------|------|
| [`rust-toolchain.toml`](rust-toolchain.toml) | Rust compiler channel and required components (`rustfmt`, `clippy`). |
| [`Cargo.lock`](Cargo.lock) | Exact resolved version of every transitive dependency. |
| [`Containerfile`](Containerfile) | OS userland, system libraries, and the toolchain installation. |

## Quick start (host)

If you already have Rust installed, the toolchain file makes `cargo`
auto-download the right version:

```bash
cargo test --workspace            # auto-installs Rust 1.94 on first run
cargo run -p klayout --example end_to_end --release
```

## Quick start (Docker)

For genuine reproducibility — same OS, same libc, same toolchain — use
the container:

```bash
# Build the image once (~3 min, ~250 MB).
docker build -t klayout-rs:dev -f Containerfile .

# Run the full test suite.
docker run --rm klayout-rs:dev

# Run the end-to-end demo, capturing the GDS output to ./out/.
mkdir -p out
docker run --rm -v "$PWD/out:/work/out" klayout-rs:dev \
    cargo run -p klayout --example end_to_end --release
ls out/
```

The image intentionally does **not** include `klayout.db` (the C++
reference engine used to generate the validation corpus). The corpus
is checked in, so CI does not need the oracle installed. If you're
regenerating the corpus, install `klayout.db` separately on the host
and run `python validation/oracle.py` there — this is the only step
in the workflow that requires the oracle.

## What the Containerfile is doing

1. Pins `rust:1.94-slim-bookworm` as the base image — same Debian
   userland and same compiler version everywhere.
2. Installs the minimum system dependencies for Cargo to resolve and
   build (no GUI libs, no Python ML stack).
3. Strips the `[patch.crates-io]` block in `Cargo.toml` that points
   `rlx-*` at `/Users/Shared/rlx`. Those patches are inert today — no
   klayout-rs crate depends on `rlx-*` — but they'd fail in-container
   path resolution. When a klayout-rs crate eventually adopts rlx,
   replace the `sed` line with a `COPY` of the relevant rlx
   sub-crate(s) into the image.
4. Warms `cargo fetch` so the test-running layer doesn't re-download
   crates on every `docker run`.

## Caveats

- **The image is not a release artifact.** It's a build environment
  for development and CI. Production deployments of any tool built on
  klayout-rs should ship statically-linked binaries from this image,
  not the image itself.
- **No GPU / hardware-accelerated paths.** klayout-rs is CPU-only
  today. When `rlx-cuda` / `rlx-metal` become consumable, the
  Containerfile will need a GPU base image and the corresponding
  toolchain (CUDA, MoltenVK, etc).
- **No klayout.db.** Installing it bumps the image to ~2 GB and
  pulls a Qt5 dependency tree; not worth it for ordinary CI.

## CI integration sketch

A `.github/workflows/ci.yml` job that uses this image:

```yaml
jobs:
  test:
    runs-on: ubuntu-latest
    container:
      image: klayout-rs:dev   # or push to ghcr.io and pull from there
    steps:
      - uses: actions/checkout@v4
      - run: cargo test --workspace --no-fail-fast
      - run: cargo clippy --workspace --all-targets -- -D warnings
      - run: cargo fmt --all -- --check
```

The corresponding workflow isn't checked in yet (see "what's next?"
in the project's planning notes); when it lands, this image is what
it will run inside.
