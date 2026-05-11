#!/usr/bin/env python3
"""Compare two benchmark snapshots (JSON). Exit 1 on regressions."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


def _load(p: Path) -> dict[str, Any]:
    return json.loads(p.read_text())


def _compare_parity(old: dict, new: dict) -> list[str]:
    errs = []
    o = old.get("sections", {}).get("parity_report", {}).get("overall", {})
    n = new.get("sections", {}).get("parity_report", {}).get("overall", {})
    if not o or not n:
        return ["missing parity overall in baseline or candidate"]
    if n.get("total", 0) < o.get("total", 0):
        errs.append(
            f"parity total cases shrunk: {o.get('total')} -> {n.get('total')}"
        )
    if n.get("pct", 0) + 1e-6 < o.get("pct", 0):
        errs.append(
            f"parity pass rate dropped: {o['pct']}% -> {n['pct']}%"
        )
    return errs


def _compare_counts(old: dict, new: dict) -> list[str]:
    errs = []
    ow = old.get("sections", {}).get("cargo_test_workspace", {})
    nw = new.get("sections", {}).get("cargo_test_workspace", {})
    if ow.get("passed") is not None and nw.get("passed") is not None:
        if nw["passed"] < ow["passed"]:
            errs.append(f"passed tests shrunk: {ow['passed']} -> {nw['passed']}")
        if nw.get("failed", 0) > ow.get("failed", 0):
            errs.append(
                f"failed tests grew: {ow.get('failed', 0)} -> {nw['failed']}"
            )
    return errs


def _compare_docker_fp(old: dict, new: dict) -> list[str]:
    errs = []
    od = old.get("sections", {}).get("docker", {})
    nd = new.get("sections", {}).get("docker", {})
    if od.get("exit_code") == 0:
        ogs = od.get("gcd_synth") or {}
        ngs = nd.get("gcd_synth") or {}
        if ogs.get("synth_odb_sha20") and ngs.get("synth_odb_sha20"):
            if ogs["synth_odb_sha20"] != ngs["synth_odb_sha20"]:
                errs.append(
                    "gcd synth ODB fingerprint changed "
                    f"({ogs['synth_odb_sha20']} -> {ngs['synth_odb_sha20']}); "
                    "expected if OpenROAD/ORFS toolchain updated"
                )
    return errs


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("baseline", type=Path, help="e.g. validation/benchmark_snapshots/latest.json")
    ap.add_argument("candidate", type=Path)
    args = ap.parse_args()
    old_d = _load(args.baseline)
    new_d = _load(args.candidate)
    messages = []
    messages.extend(_compare_parity(old_d, new_d))
    messages.extend(_compare_counts(old_d, new_d))
    messages.extend(_compare_docker_fp(old_d, new_d))

    fatal = [
        m
        for m in messages
        if not (
            "fingerprint changed" in m or "toolchain updated" in m
        )
    ]

    tool_changed = [
        m
        for m in messages
        if "fingerprint changed" in m or "toolchain updated" in m
    ]

    for m in fatal:
        print(f"FAIL {m}", file=sys.stderr)
    for m in tool_changed:
        print(f"NOTICE {m}", file=sys.stderr)

    if fatal:
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
