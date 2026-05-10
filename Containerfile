# Reproducible build environment for klayout-rs.
#
# Build:
#   docker build -t klayout-rs:dev -f Containerfile .
#
# Verify the workspace builds + tests cleanly in a known-good environment:
#   docker run --rm klayout-rs:dev cargo test --workspace
#
# Run the end-to-end demo:
#   docker run --rm -v "$PWD/out:/work/out" klayout-rs:dev \
#     cargo run -p klayout --example end_to_end --release
#   ls out/  # GDS lands here
#
# This image deliberately does NOT include klayout.db (the C++
# reference engine) — the validation corpus is checked in, so CI
# verifies parity without needing the oracle installed. To regenerate
# the corpus, install klayout.db separately and run
# `python validation/oracle.py` outside the container.

# Pin to the same channel `rust-toolchain.toml` declares so a cargo
# command inside the container uses the exact compiler the maintainers
# tested with. Debian-slim base keeps the image under 200 MB.
FROM rust:1.94-slim-bookworm

# Build-time deps:
#   - pkg-config / libssl-dev: required by some transitively-pulled crates
#     during Cargo.lock resolution.
#   - python3 / python3-pip: needed for `validation/oracle*.py` regen
#     (kept thin; klayout.db itself is not installed).
#   - git: cargo features that consult VCS metadata.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        git \
        pkg-config \
        libssl-dev \
        python3 \
        python3-pip \
 && rm -rf /var/lib/apt/lists/*

# Pre-install rustfmt + clippy so `cargo clippy` and `cargo fmt --check`
# work without a network round trip.
RUN rustup component add rustfmt clippy

WORKDIR /work
COPY . .

# Strip the `[patch.crates-io]` block targeting /Users/Shared/rlx —
# those paths don't exist inside the container, and no klayout-rs crate
# currently depends on rlx-* anyway, so the patches are inert. The sed
# below removes the block in place; if the workspace ever adds a real
# rlx-* dep, replace this with a `COPY rlx /opt/rlx` and rewrite the
# patch paths to `/opt/rlx/...`.
RUN sed -i '/^\[patch\.crates-io\]/,/^$/d' Cargo.toml

# Warm the dependency cache so subsequent `docker run` invocations
# don't re-download every crate.
RUN cargo fetch --locked || cargo fetch

# Default command: full workspace test. Override with `docker run ...`
# to do something else.
CMD ["cargo", "test", "--workspace", "--no-fail-fast"]
