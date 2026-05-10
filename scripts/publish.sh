#!/usr/bin/env bash
# scripts/publish.sh — slow, tiered publisher for the klayout-rs workspace.
#
# Default mode is dry-run: traces the per-tier publish plan with zero
# delays, then runs a single `cargo publish --workspace --dry-run` at
# the end. That single command validates every tarball against a temp
# registry without uploading anything to crates.io — gentle on the
# registry infrastructure.
#
# Real publish:
#
#     DRY_RUN=0 ./scripts/publish.sh
#
# In live mode the script publishes one crate at a time and sleeps
# between crates and between tiers so crates.io has time to index
# each crate before its dependents try to resolve it.
#
# Knobs (env-overridable):
#
#     DRY_RUN            1 (default) — trace + workspace dry-run
#                        0           — actually publish to crates.io
#     TOOLCHAIN          1.94.0 (default) — passed as `cargo +TOOLCHAIN`.
#                        Earlier toolchains choke on the [patch.crates-io]
#                        block that points at the rlx workspace.
#     INTRA_TIER_DELAY   seconds between crates within a tier (default 20).
#                        Each upload takes a few seconds to land in the
#                        sparse index; this leaves headroom.
#     INTER_TIER_DELAY   seconds between tiers (default 90). Dependents
#                        in the next tier won't resolve until the
#                        previous tier's crates are visible on crates.io.
#
# Run from the workspace root (or anywhere — the script just calls
# cargo, which finds the workspace via $PWD).

set -euo pipefail

# ── Tunables ────────────────────────────────────────────────────
TOOLCHAIN="${TOOLCHAIN:-1.94.0}"
DRY_RUN="${DRY_RUN:-1}"
INTRA_TIER_DELAY="${INTRA_TIER_DELAY:-20}"
INTER_TIER_DELAY="${INTER_TIER_DELAY:-90}"

# ── Topological tiers ───────────────────────────────────────────
# Within a tier the order doesn't matter for correctness — every
# crate in tier N depends only on crates in tiers < N.
TIER_1_LABEL="Tier 1: foundation"
TIER_1=(klayout-core)

TIER_2_LABEL="Tier 2: direct core consumers"
TIER_2=(klayout-spatial klayout-io klayout-pdk klayout-place klayout-cts klayout-route klayout-lef klayout-liberty)

TIER_3_LABEL="Tier 3: spatial / liberty consumers"
TIER_3=(klayout-geom klayout-sta)

TIER_4_LABEL="Tier 4: geom consumers"
TIER_4=(klayout-drc klayout-connect)

TIER_5_LABEL="Tier 5: drc consumer"
TIER_5=(klayout-deck)

TIER_6_LABEL="Tier 6: prelude umbrella"
TIER_6=(klayout)

# ── Helpers ─────────────────────────────────────────────────────
log() { printf '\033[1;34m[publish]\033[0m %s\n' "$*"; }

# In dry-run mode all sleeps are zero so the trace runs instantly.
nap() {
    local secs=$1
    if [[ "$DRY_RUN" == "0" && "$secs" -gt 0 ]]; then
        log "  sleeping ${secs}s …"
        sleep "$secs"
    fi
}

publish_one() {
    local crate=$1
    if [[ "$DRY_RUN" == "1" ]]; then
        printf '  • %-22s  (would publish; dry-run trace)\n' "$crate"
    else
        log "publishing $crate"
        cargo "+${TOOLCHAIN}" publish -p "$crate" --allow-dirty
    fi
}

publish_tier() {
    local label=$1
    shift
    local crates=("$@")
    log "── ${label}  [${#crates[@]} crate(s)] ──"
    local i=0
    for c in "${crates[@]}"; do
        publish_one "$c"
        ((++i))
        if (( i < ${#crates[@]} )); then
            nap "$INTRA_TIER_DELAY"
        fi
    done
}

# ── Banner ──────────────────────────────────────────────────────
log "toolchain: cargo +${TOOLCHAIN}"
if [[ "$DRY_RUN" == "1" ]]; then
    log "mode:      DRY-RUN  (no uploads, sleeps skipped)"
else
    log "mode:      LIVE     (will upload to crates.io)"
    log "delays:    intra-tier ${INTRA_TIER_DELAY}s, inter-tier ${INTER_TIER_DELAY}s"
    log "WARNING: this WILL publish to crates.io. Sleeping 5s — Ctrl-C to abort."
    sleep 5
fi

# ── Drive each tier ─────────────────────────────────────────────
publish_tier "$TIER_1_LABEL" "${TIER_1[@]}"; nap "$INTER_TIER_DELAY"
publish_tier "$TIER_2_LABEL" "${TIER_2[@]}"; nap "$INTER_TIER_DELAY"
publish_tier "$TIER_3_LABEL" "${TIER_3[@]}"; nap "$INTER_TIER_DELAY"
publish_tier "$TIER_4_LABEL" "${TIER_4[@]}"; nap "$INTER_TIER_DELAY"
publish_tier "$TIER_5_LABEL" "${TIER_5[@]}"; nap "$INTER_TIER_DELAY"
publish_tier "$TIER_6_LABEL" "${TIER_6[@]}"

# ── Workspace verify (dry-run only) ─────────────────────────────
if [[ "$DRY_RUN" == "1" ]]; then
    log ""
    log "verifying tarballs with a single workspace dry-run …"
    log "(uses a temp registry — no uploads, gentle on crates.io)"
    cargo "+${TOOLCHAIN}" publish --workspace --dry-run --allow-dirty --exclude klayout-validate
fi

log ""
log "done."
