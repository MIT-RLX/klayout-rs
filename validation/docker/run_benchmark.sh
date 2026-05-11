#!/usr/bin/env bash
# Docker-only PDK + OpenROAD benchmark smoke tests.
#
# Prerequisites: Docker (linux/amd64 — may use Rosetta/virtualization on Apple Silicon).
#
# Usage (from repo root):
#   ./validation/docker/run_benchmark.sh build-image
#   ./validation/docker/run_benchmark.sh pdk-smoke          # curl Sky130 HD + ingest in OpenDB
#   ./validation/docker/run_benchmark.sh openroad-gcd-synth # ORFS gcd through synthesis (~tens of s)
#   ./validation/docker/run_benchmark.sh all
#
# Override image name:
#   KLAYOUT_RS_ORACLE_IMAGE=mytag:latest ./validation/docker/run_benchmark.sh pdk-smoke
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
IMAGE="${KLAYOUT_RS_ORACLE_IMAGE:-klayout-rs-oracle:latest}"
# Pinned KLayout-from-source image (see Dockerfile.klayout + klayout-git-ref).
KLAYOUT_IMAGE="${KLAYOUT_RS_KLAYOUT_IMAGE:-klayout-rs-klayout:latest}"

SKY130_TLEF_URL_DEFAULT="https://raw.githubusercontent.com/google/skywater-pdk-libs-sky130_fd_sc_hd/main/tech/sky130_fd_sc_hd.tlef"
SKY130_INV_LEF_URL_DEFAULT="https://raw.githubusercontent.com/google/skywater-pdk-libs-sky130_fd_sc_hd/main/cells/inv/sky130_fd_sc_hd__inv_1.lef"

_die() {
  printf '%s\n' "run_benchmark.sh: $*" >&2
  exit 1
}

cmd_help() {
  cat <<EOF
Docker benchmark smoke (Sky130 ingest + bundled OpenROAD-flow gcd synth).

Commands:
  build-image           docker build validation/docker -> ${IMAGE} (OpenROAD oracle)
  build-klayout-image   docker build Dockerfile.klayout -> ${KLAYOUT_IMAGE} (pinned KLayout git)
  klayout-smoke         import klayout.db inside ${KLAYOUT_IMAGE}
  klayout-oracle-drc    regenerate drc.json via pinned KLayout into a tmp dir, diff vs repo
  klayout-benchmark     klayout-smoke + klayout-oracle-drc
  pdk-smoke             download tech + inverter LEF, run OpenROAD ingest
  openroad-gcd-synth    make synth for designs/sky130hd/gcd (PDK bundled in ORFS image)
  all                   pdk-smoke then openroad-gcd-synth

Environment:
  KLAYOUT_RS_ORACLE_IMAGE   docker image tag (default: klayout-rs-oracle:latest)
  KLAYOUT_RS_KLAYOUT_IMAGE  pinned KLayout image (default: klayout-rs-klayout:latest)
  SKY130_TLEF_URL           override tech LEF URL
  SKY130_INV_LEF_URL        override inverter macro LEF URL
EOF
}

cmd_build_image() {
  docker build -t "$IMAGE" "$SCRIPT_DIR"
}

cmd_build_klayout_image() {
  docker build --platform linux/amd64 \
    -f "$SCRIPT_DIR/Dockerfile.klayout" \
    -t "$KLAYOUT_IMAGE" \
    "$SCRIPT_DIR"
}

_require_klayout_image() {
  if ! docker image inspect "$KLAYOUT_IMAGE" >/dev/null 2>&1; then
    _die "KLayout image '${KLAYOUT_IMAGE}' not found — run: $0 build-klayout-image"
  fi
}

cmd_klayout_smoke() {
  _require_klayout_image
  docker run --rm --platform linux/amd64 "$KLAYOUT_IMAGE" \
    python3 -c 'import klayout.db as d; print("KLayout_py", getattr(d, "__version__", "unknown"))'
}

cmd_klayout_oracle_drc() {
  _require_klayout_image
  command -v diff >/dev/null 2>&1 || _die "diff not found"
  local ROOT
  ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
  local work
  work="$(mktemp -d)"
  trap "rm -rf \"${work}\"" EXIT

  # ORACLE_CORPUS_DIR: see validation/oracle.py — writes only under /oracle-out.
  if ! out="$(docker run --rm --platform linux/amd64 \
    -v "$ROOT:/work:ro" \
    -v "$work:/oracle-out" \
    -e ORACLE_CORPUS_DIR=/oracle-out \
    "$KLAYOUT_IMAGE" \
    bash -lc 'set -euo pipefail; python3 /work/validation/oracle.py drc && diff -q /oracle-out/drc.json /work/validation/corpus/drc.json' 2>&1)"; then
    printf '%s\n' "$out" >&2
    _die "klayout oracle drc corpus drift (rebuild image after editing klayout-git-ref, or refresh corpus)"
  fi
  printf '%s\n' "$out"
  printf '%s\n' "ok: klayout-oracle-drc matches validation/corpus/drc.json"
}

cmd_klayout_benchmark() {
  cmd_klayout_smoke
  cmd_klayout_oracle_drc
}

_require_image() {
  if ! docker image inspect "$IMAGE" >/dev/null 2>&1; then
    _die "Docker image '${IMAGE}' not found — run: $0 build-image  (or export KLAYOUT_RS_ORACLE_IMAGE=openroad/orfs:latest if you already pulled ORFS)"
  fi
}

cmd_pdk_smoke() {
  _require_image
  command -v curl >/dev/null 2>&1 || _die "curl not found — install curl or prefetch files manually"
  work="$(mktemp -d)"
  trap "rm -rf \"${work}\"" EXIT

  curl -fsSL "${SKY130_TLEF_URL:-$SKY130_TLEF_URL_DEFAULT}" \
    -o "$work/sky130_fd_sc_hd.tlef"
  curl -fsSL "${SKY130_INV_LEF_URL:-$SKY130_INV_LEF_URL_DEFAULT}" \
    -o "$work/sky130_cell.lef"

  # Mount repo read-only so we use the checked-in TCL without rebaking the image.
  ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
  if ! out="$(docker run --rm --platform linux/amd64 \
    -v "$work:/work" \
    -v "$ROOT:/repo:ro" \
    "$IMAGE" openroad -no_init /repo/validation/docker/tcl/pdk_smoke.tcl 2>&1)"; then
    printf '%s\n' "$out" >&2
    _die "docker / openroad failed (see output above)"
  fi
  printf '%s\n' "$out"
  echo "$out" | grep -q "PDK_SMOKE masters_loaded=" \
    || _die "OpenROAD did not report PDK_SMOKE masters_loaded= (check OpenDB ingest)"
  printf '%s\n' "ok: pdk-smoke (Sky130 HD tech + sky130_fd_sc_hd__inv_1)"
}

cmd_openroad_gcd_synth() {
  _require_image
  docker run --rm --platform linux/amd64 "$IMAGE" bash -lc \
    'set -euo pipefail
     cd /OpenROAD-flow-scripts/flow
     make DESIGN_CONFIG=designs/sky130hd/gcd/config.mk synth'
  printf '%s\n' "ok: openroad-gcd-synth (ORFS sky130hd/gcd make synth)"
}

cmd_all() {
  cmd_pdk_smoke
  cmd_openroad_gcd_synth
}

main() {
  case "${1:-}" in
  build-image) cmd_build_image ;;
  build-klayout-image) cmd_build_klayout_image ;;
  klayout-smoke) cmd_klayout_smoke ;;
  klayout-oracle-drc) cmd_klayout_oracle_drc ;;
  klayout-benchmark) cmd_klayout_benchmark ;;
  pdk-smoke) cmd_pdk_smoke ;;
  openroad-gcd-synth | gcd-synth) cmd_openroad_gcd_synth ;;
  all) cmd_all ;;
  "" | help | -h | --help) cmd_help ;;
  *) _die "unknown command: ${1:-} (run with no args for help)" ;;
  esac
}

main "$@"
