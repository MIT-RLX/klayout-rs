#!/usr/bin/env python3
"""Run workspace checks + parity + Docker smoke + Criterion benches; write JSON snapshot.

Default profile is **full** (all RSMT sizes including n=256; long rsmt timeout).

CLI / env overrides:
  --fast / RECORD_BENCHMARK_FAST       Shorter Criterion timings; RSMT benches only 4|16|64.
  --strict-klayout / RECORD_BENCHMARK_STRICT_KLAYOUT   Exit 1 if the KLayout oracle docker
                                   phase is skipped (e.g. image missing). Build:
                                   ./validation/docker/run_benchmark.sh build-klayout-image
  RECORD_BENCHMARK_QUIET=1         Disable stderr progress lines.
"""

from __future__ import annotations

import datetime as _dt
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[2]
SNAP_ROOT = ROOT / "validation" / "benchmark_snapshots"
RAW_DIR = SNAP_ROOT / "raw"
DOCKER_SCRIPT = ROOT / "validation" / "docker" / "run_benchmark.sh"

SCHEMA_VERSION = 2

_BENCH_PACKAGES: list[tuple[str, str]] = [
    ("klayout-drc", "width"),
    ("klayout-place", "quadratic"),
    ("klayout-route", "pathfinder"),
    ("klayout-route", "rsmt"),
]


def _parse_snapshot_cli() -> tuple[bool, bool]:
    """Return (profile_fast, strict_klayout). Default profile is **full** (includes RSMT n=256)."""
    args = sys.argv[1:]
    profile_fast = "--fast" in args or os.environ.get("RECORD_BENCHMARK_FAST", "").lower() in (
        "1",
        "true",
        "yes",
        "on",
    )
    strict_klayout = "--strict-klayout" in args or os.environ.get(
        "RECORD_BENCHMARK_STRICT_KLAYOUT", ""
    ).lower() in ("1", "true", "yes", "on")
    return profile_fast, strict_klayout


def _criterion_shell_args(profile_fast: bool) -> list[str]:
    """Arguments passed after `cargo bench … -- ` before optional per-bench suffix."""
    if profile_fast:
        return ["--sample-size", "15", "--warm-up-time", "0.25", "--measurement-time", "0.35"]
    return ["--sample-size", "20", "--warm-up-time", "0.35", "--measurement-time", "0.55"]


def _per_bench_suffix(pkg: str, bench: str, profile_fast: bool) -> list[str]:
    """RSMT: `--noplot`; fast slice omits rsmt/256 (can exceed ~25 min locally)."""
    if (pkg, bench) == ("klayout-route", "rsmt"):
        return ["--noplot", r"rsmt/(4|16|64)"] if profile_fast else ["--noplot"]
    return []


def _bench_timeout_s(pkg: str, bench: str, profile_fast: bool) -> float:
    """Subprocess timeout for one `cargo bench` invocation (seconds)."""
    if (pkg, bench) == ("klayout-route", "rsmt"):
        return 7200.0 if profile_fast else 28800.0
    return 9000.0


def _progress_enabled() -> bool:
    q = os.environ.get("RECORD_BENCHMARK_QUIET", "").lower().strip()
    return q not in ("1", "true", "yes", "on")


def _progress_bar(done: int, total: int, width: int = 22) -> str:
    if total <= 0:
        return "[" + "=" * width + "]"
    fill = max(0, min(width, int(round(done * width / total))))
    return "[" + ("=" * fill) + ("-" * (width - fill)) + "]"


def _progress_emit(done: int, total: int, title: str, detail: str = "") -> None:
    if not _progress_enabled():
        return
    total = max(1, total)
    pct = 100.0 * done / total
    msg = f"{_progress_bar(done, total)} {pct:6.2f}%  [{done}/{total}]  {title}"
    if detail:
        msg = f"{msg}  —  {detail}"
    print(msg, file=sys.stderr, flush=True)


def _run(
    argv: list[str],
    cwd: Path | None = None,
    env: dict[str, str] | None = None,
    timeout: float | None = None,
) -> tuple[int, str, str]:
    merged = os.environ.copy()
    if env:
        merged.update(env)
    r = subprocess.run(
        argv,
        cwd=cwd or ROOT,
        capture_output=True,
        text=True,
        timeout=timeout,
        env=merged,
    )
    return r.returncode, r.stdout or "", r.stderr or ""


def _parse_test_result(summary: str) -> dict[str, Any]:
    """Cargo prints one `test result:` line per harness; aggregate counts."""
    pat = re.compile(
        r"test result:\s*\w+\.\s*(\d+)\s*passed;\s*(\d+)\s*failed;\s*(\d+)\s*ignored",
        re.MULTILINE | re.IGNORECASE,
    )
    passed = failed = ignored = 0
    for m in pat.finditer(summary):
        passed += int(m.group(1))
        failed += int(m.group(2))
        ignored += int(m.group(3))
    status = "ok" if failed == 0 else "failed"
    return {
        "status": status,
        "passed": passed,
        "failed": failed,
        "ignored": ignored,
    }


def _parse_parity_overall(text: str) -> dict[str, Any]:
    """Extract **Overall:** N / N cases passing (P%) from parity_report stdout."""
    m = re.search(
        r"\*\*Overall:\*\*\s+(\d+)\s+/\s+(\d+)\s+cases passing\s+\(([\d.]+)%\)",
        text,
    )
    if not m:
        return {}
    return {
        "passing": int(m.group(1)),
        "total": int(m.group(2)),
        "pct": float(m.group(3)),
    }


def _parse_pdk_smoke(stdout: str) -> dict[str, Any]:
    m = re.search(r"PDK_SMOKE\s+masters_loaded=(\d+)", stdout)
    if not m:
        return {"ok": False}
    return {"ok": True, "masters_loaded": int(m.group(1))}


def _extract_gcd_odb_sha(gcd_stdout: str) -> str | None:
    """ORFS log line: `1_synth   <elapsed>  <mem>  <sha1sum .odb [0:20)>`"""
    for line in gcd_stdout.splitlines():
        parts = line.split()
        if len(parts) >= 4 and parts[0] == "1_synth":
            return parts[-1]
    m = re.search(r"\b([0-9a-f]{20})\b", gcd_stdout)
    return m.group(1) if m else None


def _docker_image_exists(name: str) -> bool:
    code, _, _ = _run(["docker", "image", "inspect", name], timeout=60)
    return code == 0


def _median_from_time_bracket(bracket: str) -> str | None:
    """Criterion `time: [low med high]` — each value is `float` + unit (e.g. `4.92 µs`)."""
    parts = re.findall(r"[\d.]+\s*(?:µs|ms|ns|s)\b", bracket)
    if len(parts) >= 2:
        return parts[1].strip()
    if len(parts) == 1:
        return parts[0].strip()
    toks = bracket.split()
    if len(toks) >= 2:
        return f"{toks[1]} {toks[2]}" if toks[2] in ("µs", "ms", "ns", "s") else toks[1]
    return None


def _parse_criterion_medians(text: str) -> dict[str, str]:
    """Map benchmark id string -> median time string (value + unit)."""
    out: dict[str, str] = {}
    lines = text.splitlines()
    # One-line form: `rsmt/4                  time:   [...]`
    one_line = re.compile(r"^(\S+)\s+time:\s+\[([^\]]+)\]")
    for line in lines:
        om = one_line.match(line.strip())
        if om:
            med = _median_from_time_bracket(om.group(2))
            if med:
                out[om.group(1)] = med

    # Two-line header: benchmark id alone, then indented `time:` on the next line.
    id_line = re.compile(r"^[A-Za-z0-9_/.\-]+(?:/[A-Za-z0-9_/.\-]+)*\s*$")
    for idx in range(len(lines) - 1):
        stripped = lines[idx].strip()
        if (
            not stripped
            or stripped.startswith(
                ("Benchmarking ", "running ", "Found ", "Warning", "change:", "thrpt:")
            )
            or "time:" in stripped
        ):
            continue
        if not id_line.match(stripped):
            continue
        m = re.search(r"time:\s+\[([^\]]+)\]", lines[idx + 1])
        if not m:
            continue
        med = _median_from_time_bracket(m.group(1))
        if med:
            out[stripped] = med

    i = 0
    while i < len(lines):
        line = lines[i]
        if line.startswith("Benchmarking "):
            name = line.split("Benchmarking ", 1)[1].strip()
            j = i + 1
            while j < len(lines) and not lines[j].strip().startswith("time:"):
                j += 1
            if j < len(lines):
                m = re.search(r"time:\s+\[([^\]]+)\]", lines[j])
                if m:
                    med = _median_from_time_bracket(m.group(1))
                    if med:
                        out[name] = med
            i = j + 1
        else:
            i += 1
    return out


def main() -> int:
    RAW_DIR.mkdir(parents=True, exist_ok=True)
    stamp = _dt.datetime.now(_dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    profile_fast, strict_klayout = _parse_snapshot_cli()
    criterion_argv = _criterion_shell_args(profile_fast)

    # Phases: workspace + parity + ORFS docker + klayout docker + benches + write/final
    n_benches = len(_BENCH_PACKAGES)
    phases_total = 2 + n_benches + 2 + 1

    rustc_code, rustc_out, rustc_err = _run(["rustc", "-vV"])
    git_code, git_sha, _ = _run(["git", "rev-parse", "HEAD"])

    snap: dict[str, Any] = {
        "schema_version": SCHEMA_VERSION,
        "recorded_at": stamp,
        "snapshot_profile": "fast" if profile_fast else "full",
        "strict_klayout_requested": strict_klayout,
        "git_sha": (git_sha.strip() if git_code == 0 else None),
        "rustc": rustc_out.strip().splitlines()[0] if rustc_out.strip() else None,
        "rustc_verbose": rustc_out.strip() if rustc_code == 0 else rustc_err.strip() or None,
    }

    sections: dict[str, Any] = {}
    phase_done = 0

    if _progress_enabled():
        host = rustc_out.strip().splitlines()[0] if rustc_out.strip() else "rustc (?)"
        print(
            f"benchmark snapshot ({stamp}) · {host} · {phases_total} steps · {'fast' if profile_fast else 'full'}",
            file=sys.stderr,
            flush=True,
        )

    t0 = time.perf_counter()
    code_t, out_t, err_t = _run(
        ["cargo", "test", "--workspace"],
        timeout=7200,
    )
    elapsed_t = round(time.perf_counter() - t0, 2)
    full_test = "\n".join((out_t, err_t))
    (RAW_DIR / f"cargo_test_workspace_{stamp}.txt").write_text(full_test)
    tr = _parse_test_result(full_test)
    sections["cargo_test_workspace"] = {
        "exit_code": code_t,
        "seconds": elapsed_t,
        **tr,
    }
    phase_done += 1
    _progress_emit(
        phase_done,
        phases_total,
        "cargo test --workspace",
        f"{elapsed_t}s · {tr.get('passed', '?')} passed · exit {code_t}",
    )

    t0 = time.perf_counter()
    code_p, out_p, err_p = _run(
        [
            "cargo",
            "test",
            "-p",
            "klayout-validate",
            "--test",
            "parity_report",
            "--",
            "--nocapture",
        ],
        timeout=600,
    )
    parity_text = "\n".join((out_p, err_p))
    (RAW_DIR / f"parity_report_{stamp}.txt").write_text(parity_text)
    overall = _parse_parity_overall(parity_text)
    sec_p = round(time.perf_counter() - t0, 2)
    sections["parity_report"] = {
        "exit_code": code_p,
        "seconds": sec_p,
        "overall": overall,
    }
    phase_done += 1
    po = sections["parity_report"].get("overall") or {}
    _progress_emit(
        phase_done,
        phases_total,
        "parity_report",
        f"{sec_p}s · {po.get('passing', '?')}/{po.get('total', '?')} cases · exit {code_p}",
    )

    docker_section: dict[str, Any] = {"available": shutil.which("docker") is not None}
    if docker_section["available"] and DOCKER_SCRIPT.is_file():
        t_d = time.perf_counter()
        code_d, out_d, err_d = _run(["bash", str(DOCKER_SCRIPT), "all"], timeout=7200)
        combined_d = "\n".join((out_d, err_d))
        (RAW_DIR / f"docker_benchmark_all_{stamp}.txt").write_text(combined_d)
        docker_section["exit_code"] = code_d
        docker_section["seconds"] = round(time.perf_counter() - t_d, 2)
        docker_section["pdk_smoke"] = _parse_pdk_smoke(combined_d)
        docker_section["gcd_synth"] = {
            "ok": code_d == 0,
            "synth_odb_sha20": _extract_gcd_odb_sha(combined_d),
        }
        if code_d != 0:
            docker_section["error_tail"] = combined_d[-4000:]
    else:
        docker_section["skipped"] = "docker or run_benchmark.sh not available"

    sections["docker"] = docker_section

    phase_done += 1
    dock_detail = docker_section.get("skipped")
    if dock_detail:
        dock_msg = "skipped · " + dock_detail.split(";")[0][:60]
    else:
        gcs = docker_section.get("gcd_synth") or {}
        pdk_ok = docker_section.get("pdk_smoke", {}).get("ok")
        dock_msg = (
            f'{docker_section.get("seconds")}s exit {docker_section.get("exit_code")} · '
            f"pdk={'ok' if pdk_ok else 'fail'} · gcd_odb="
            f"{gcs.get('synth_odb_sha20') or 'n/a'}"
        )
    _progress_emit(
        phase_done,
        phases_total,
        "docker run_benchmark.sh all",
        dock_msg[:120],
    )

    klayout_img = os.environ.get("KLAYOUT_RS_KLAYOUT_IMAGE", "klayout-rs-klayout:latest")
    klayout_section: dict[str, Any] = {
        "available": shutil.which("docker") is not None,
        "image": klayout_img,
    }
    if (
        klayout_section["available"]
        and DOCKER_SCRIPT.is_file()
        and _docker_image_exists(klayout_img)
    ):
        t_k = time.perf_counter()
        code_k, out_k, err_k = _run(
            ["bash", str(DOCKER_SCRIPT), "klayout-benchmark"],
            timeout=3600,
        )
        combined_k = "\n".join((out_k, err_k))
        (RAW_DIR / f"klayout_docker_benchmark_{stamp}.txt").write_text(combined_k)
        klayout_section["exit_code"] = code_k
        klayout_section["seconds"] = round(time.perf_counter() - t_k, 2)
        m_py = re.search(r"KLayout_py\s+(\S+)", combined_k)
        if m_py:
            klayout_section["python_module_version_line"] = m_py.group(1)
        if code_k != 0:
            klayout_section["error_tail"] = combined_k[-4000:]
    elif not klayout_section["available"]:
        klayout_section["skipped"] = "docker not available"
    elif not DOCKER_SCRIPT.is_file():
        klayout_section["skipped"] = "run_benchmark.sh missing"
    elif not _docker_image_exists(klayout_img):
        klayout_section["skipped"] = (
            f"image {klayout_img} not present — run "
            "./validation/docker/run_benchmark.sh build-klayout-image"
        )

    sections["klayout_docker"] = klayout_section
    phase_done += 1
    if klayout_section.get("skipped"):
        k_msg = klayout_section["skipped"][:100]
    else:
        k_msg = (
            f'{klayout_section.get("seconds")}s exit {klayout_section.get("exit_code")} · '
            f'{klayout_section.get("python_module_version_line") or "version n/a"}'
        )
    _progress_emit(phase_done, phases_total, "klayout-docker benchmark", k_msg[:120])

    criterion_runs: list[dict[str, Any]] = []
    for pkg, bench_name in _BENCH_PACKAGES:
        bench_key = f"{pkg}__{bench_name}"
        lbl = f"cargo bench -p {pkg} --bench {bench_name}"
        t0 = time.perf_counter()
        bench_argv = (
            ["cargo", "bench", "-p", pkg, "--bench", bench_name, "--"]
            + criterion_argv
            + _per_bench_suffix(pkg, bench_name, profile_fast)
        )
        bt = _bench_timeout_s(pkg, bench_name, profile_fast)
        code_b, out_b, err_b = _run(bench_argv, timeout=bt)
        elapsed_b = round(time.perf_counter() - t0, 2)
        raw_path = RAW_DIR / f"criterion_{bench_key}_{stamp}.txt"
        raw_text = "\n".join((out_b, err_b))
        raw_path.write_text(raw_text)
        meds = _parse_criterion_medians(raw_text)

        criterion_runs.append(
            {
                "package": pkg,
                "bench": bench_name,
                "exit_code": code_b,
                "seconds": elapsed_b,
                "medians_by_id": meds,
                "raw_log": str(raw_path.relative_to(ROOT)),
            }
        )
        phase_done += 1
        _progress_emit(
            phase_done,
            phases_total,
            lbl,
            f"{elapsed_b}s · exit {code_b} · {len(meds)} bench ids timed",
        )

    sections["criterion_benches"] = {
        "args": criterion_argv,
        "bench_suffix_filters": {
            "klayout-route::rsmt": _per_bench_suffix("klayout-route", "rsmt", profile_fast),
        },
        "runs": criterion_runs,
    }

    snap["sections"] = sections

    out_name = f"snapshot_{stamp}.json"
    out_path = SNAP_ROOT / out_name
    latest_path = SNAP_ROOT / "latest.json"
    out_path.write_text(json.dumps(snap, indent=2, sort_keys=True) + "\n")
    shutil.copyfile(out_path, latest_path)

    phase_done += 1
    _progress_emit(
        phase_done,
        phases_total,
        "write snapshot JSON + latest.json",
        f"'{out_name}' ({max(1, out_path.stat().st_size // 1024)} KiB)",
    )

    print(f"wrote {out_path.relative_to(ROOT)}")
    print(f"wrote {latest_path.relative_to(ROOT)}")

    if code_t != 0 or code_p != 0:
        print(
            "warning: cargo test (workspace or parity_report) exited non-zero",
            file=sys.stderr,
        )
        return 1
    if any(r["exit_code"] != 0 for r in criterion_runs):
        print("warning: one or more criterion benches failed — see raw logs", file=sys.stderr)
        return 1
    d = sections.get("docker", {})
    if d.get("available") and d.get("skipped") is None and d.get("exit_code") != 0:
        print(
            "warning: docker benchmark failed — build klayout-rs-oracle:latest "
            "(docker build -t klayout-rs-oracle:latest validation/docker/)",
            file=sys.stderr,
        )
        return 1
    klay = sections.get("klayout_docker", {})
    if (
        klay.get("available")
        and klay.get("skipped") is None
        and klay.get("exit_code") not in (None, 0)
    ):
        print(
            "warning: pinned KLayout docker benchmark failed — "
            "see raw log under validation/benchmark_snapshots/raw/",
            file=sys.stderr,
        )
        return 1
    kd = sections.get("klayout_docker") or {}
    if strict_klayout and kd.get("skipped"):
        print(
            "strict-klayout: oracle benchmark skipped — "
            f"{kd['skipped']} "
            "(build ./validation/docker/run_benchmark.sh build-klayout-image)",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
