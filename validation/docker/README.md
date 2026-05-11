# Validation oracle images + Docker benchmarks

## OpenROAD oracle image (`Dockerfile`)

Extends [`openroad/orfs`](https://hub.docker.com/r/openroad/orfs): OpenROAD, OpenSTA, OpenDB,
and the bundled `spef_dump.py` parity helper.

```bash
docker build -t klayout-rs-oracle:latest validation/docker/
```

Used by `validation/oracle_external.py` when regenerating LEF/DEF/Liberty/SPEF corpus JSON.

## Pinned KLayout-from-source image (`Dockerfile.klayout`)

Builds [**KLayout**](https://github.com/KLayout/klayout) from a **pinned git ref** (tag, branch, or SHA) on Ubuntu 22.04, Qt 5, with Python 3 bindings (`klayout.db`) installed under `/opt/klayout-install/pymod`.

- **Pin:** edit the single line in [`klayout-git-ref`](klayout-git-ref) (default matches `v0.30.8` used for corpus generation).
- **Image tag:** override with `KLAYOUT_RS_KLAYOUT_IMAGE` (default `klayout-rs-klayout:latest`).

```bash
docker build --platform linux/amd64 \
  -f validation/docker/Dockerfile.klayout \
  -t klayout-rs-klayout:latest \
  validation/docker
```

First build is **slow** (full compile). Use `linux/amd64` for parity with CI and the ORFS image.

### Oracle + corpus check inside the image

`validation/oracle.py` honours `ORACLE_CORPUS_DIR` so we can regenerate JSON on a writable mount and **`diff`** against the repo without mutating it:

```bash
./validation/docker/run_benchmark.sh build-klayout-image
./validation/docker/run_benchmark.sh klayout-benchmark   # smoke import + drc.json parity
```

## PDK + OpenROAD smoke (`run_benchmark.sh`)

Runs entirely in Docker (`linux/amd64`; Apple Silicon typically uses emulation).

```bash
./validation/docker/run_benchmark.sh build-image           # ORFS oracle
./validation/docker/run_benchmark.sh build-klayout-image     # pinned KLayout (optional)
./validation/docker/run_benchmark.sh pdk-smoke             # SkyWater sky130_hd + OpenDB ingest
./validation/docker/run_benchmark.sh openroad-gcd-synth      # ORFS gcd through `make synth`
./validation/docker/run_benchmark.sh klayout-benchmark       # KLayout smoke + drc corpus diff
./validation/docker/run_benchmark.sh all                   # pdk-smoke + gcd-synth only
```

- **pdk-smoke** downloads the Apache-licensed [`skywater-pdk-libs-sky130_fd_sc_hd`](https://github.com/google/skywater-pdk-libs-sky130_fd_sc_hd)
  tech LEF (`tech/sky130_fd_sc_hd.tlef`) plus `cells/inv/sky130_fd_sc_hd__inv_1.lef`,
  mounts them into OpenROAD, and asserts `PDK_SMOKE masters_loaded=` appears in stdout.
  Override URLs with `SKY130_TLEF_URL` / `SKY130_INV_LEF_URL`.
- **openroad-gcd-synth** uses the Sky130HD platform files already shipped inside the ORFS image
  (`/OpenROAD-flow-scripts/flow/platforms/sky130hd`) and runs
  `make DESIGN_CONFIG=designs/sky130hd/gcd/config.mk synth`.

These are **integration smokes**, not checked-in differential corpus cases. Default `cargo test` stays
free of Docker and network.

### Benchmark snapshot (`record_benchmark_snapshot.py`)

`python validation/scripts/record_benchmark_snapshot.py` runs, in order: `cargo test --workspace`,
`parity_report`, **ORFS** `run_benchmark.sh all`, **pinned KLayout** `klayout-benchmark` (if the KLayout
image exists), then Criterion benches. If the KLayout image is not built, that step is skipped and the
snapshot still records a `klayout_docker.skipped` reason.

## Environment variables

| Variable | Default | Purpose |
|----------|---------|---------|
| `KLAYOUT_RS_ORACLE_IMAGE` | `klayout-rs-oracle:latest` | ORFS-based image |
| `KLAYOUT_RS_KLAYOUT_IMAGE` | `klayout-rs-klayout:latest` | Pinned KLayout build |
