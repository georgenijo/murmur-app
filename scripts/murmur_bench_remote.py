#!/usr/bin/env python3
"""Run baseline and candidate Murmur refs on one trusted Fleet Mac.

This helper is intended to be copied to the benchmark Mac and invoked there.
It creates detached temporary worktrees, shares release Cargo target caches
between refs with identical lockfiles, runs both refs against the same private
corpus, writes local reports, compares them, and removes only the temporary
worktrees it created.
"""

from __future__ import annotations

import argparse
import datetime as dt
import fcntl
import hashlib
import json
import math
import os
from pathlib import Path
import re
import signal
import shutil
import subprocess
import sys
import tempfile
import time
from typing import Any, Callable, TextIO


EXTERNAL_BINARIES = (
    "murmur-capture-agent-aarch64-apple-darwin",
    "murmur-capture-helper-aarch64-apple-darwin",
    "murmur-capture-worker-aarch64-apple-darwin",
    "murmur-llm-sidecar-aarch64-apple-darwin",
)
CACHE_POLICY = "conditioned-timed-path-v2"
CONDITIONING_PRESET = "quick"
CONDITIONING_STAGES = (
    "all-selected-quick",
    "shared-init-targets-quick-x2",
)
TARGET_CONDITIONING_PASSES = 2
DEFAULT_IDLE_TIMEOUT_SECONDS = 300.0
DEFAULT_IDLE_INTERVAL_SECONDS = 5.0
DEFAULT_IDLE_CPU_PERCENT = 20.0
DEFAULT_IDLE_SAMPLES = 3
PROCESS_MONITOR_INTERVAL_SECONDS = 1.0
CONFLICTING_PROCESS_NAMES = frozenset({"cargo", "rustc"})


def run(command: list[str], *, cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=cwd, env=env, check=True)


def output(command: list[str], *, cwd: Path | None = None) -> str:
    return subprocess.run(
        command, cwd=cwd, check=True, capture_output=True, text=True
    ).stdout.strip()


def safe_label(value: str) -> str:
    label = re.sub(r"[^A-Za-z0-9._-]+", "-", value).strip("-.")
    return (label or "ref")[:80]


def resolve_ref(repo: Path, ref: str) -> str:
    return output(["git", "rev-parse", "--verify", f"{ref}^{{commit}}"], cwd=repo)


def benchmark_environment(cache_root: Path, lockfile: Path) -> dict[str, str]:
    environment = os.environ.copy()
    environment["PATH"] = f"/opt/homebrew/bin:{environment.get('PATH', '')}"
    environment.setdefault("MACOSX_DEPLOYMENT_TARGET", "14.0")
    environment.setdefault("CMAKE_OSX_DEPLOYMENT_TARGET", "14.0")
    environment.setdefault("CMAKE_C_FLAGS", "-march=armv8.5-a")
    environment.setdefault("CMAKE_CXX_FLAGS", "-march=armv8.5-a")
    environment.setdefault(
        "RUSTFLAGS",
        "-L native=/Applications/Xcode.app/Contents/Developer/Toolchains/"
        "XcodeDefault.xctoolchain/usr/lib/clang/17/lib/darwin",
    )
    lock_digest = hashlib.sha256(lockfile.read_bytes()).hexdigest()[:16]
    environment["CARGO_TARGET_DIR"] = str(cache_root / "cargo-target" / lock_digest)
    return environment


def process_table() -> dict[int, tuple[int, str]]:
    processes = output(["/bin/ps", "-A", "-o", "pid=,ppid=,comm="])
    table: dict[int, tuple[int, str]] = {}
    for line in processes.splitlines():
        fields = line.strip().split(maxsplit=2)
        if len(fields) != 3:
            continue
        try:
            pid = int(fields[0])
            parent_pid = int(fields[1])
        except ValueError:
            continue
        table[pid] = (parent_pid, Path(fields[2]).name)
    return table


def descendant_pids(
    processes: dict[int, tuple[int, str]], root_pid: int
) -> set[int]:
    descendants = {root_pid}
    changed = True
    while changed:
        changed = False
        for pid, (parent_pid, _) in processes.items():
            if parent_pid in descendants and pid not in descendants:
                descendants.add(pid)
                changed = True
    return descendants


def conflicting_processes(
    processes: dict[int, tuple[int, str]], *, allowed_root_pid: int | None = None
) -> list[str]:
    allowed = (
        descendant_pids(processes, allowed_root_pid)
        if allowed_root_pid is not None
        else set()
    )
    return sorted(
        f"{pid}:{name}"
        for pid, (_, name) in processes.items()
        if (
            name in CONFLICTING_PROCESS_NAMES
            or name.endswith("headless_benchmark")
        )
        and pid not in allowed
    )


def terminate_process_group(process: subprocess.Popen[Any]) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=10.0)
    except ProcessLookupError:
        return
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait()


def host_snapshot() -> dict[str, Any]:
    battery = output(["/usr/bin/pmset", "-g", "batt"])
    power_match = re.search(r"Now drawing from '([^']+)'", battery)
    power_source = power_match.group(1) if power_match else "unknown"

    custom = output(["/usr/bin/pmset", "-g", "custom"])
    low_power_match = re.search(r"^\s*lowpowermode\s+(\d+)\s*$", custom, re.MULTILINE)
    low_power_mode = None if low_power_match is None else low_power_match.group(1) != "0"

    thermal = output(["/usr/bin/pmset", "-g", "therm"])
    thermal_state = (
        "nominal"
        if "No thermal warning level has been recorded" in thermal
        and "No performance warning level has been recorded" in thermal
        else "warning"
    )

    cpu_values = output(["/bin/ps", "-A", "-o", "%cpu="])
    total_cpu = sum(float(value) for value in cpu_values.split() if value)
    processor_count = os.cpu_count() or 1
    normalized_cpu = total_cpu / processor_count
    conflicts = conflicting_processes(process_table())
    return {
        "capturedAt": dt.datetime.now(dt.timezone.utc).isoformat(),
        "powerSource": power_source,
        "lowPowerMode": low_power_mode,
        "thermalState": thermal_state,
        "normalizedCpuPercent": round(normalized_cpu, 3),
        "loadAverage1m": round(os.getloadavg()[0], 3),
        "conflictingProcesses": conflicts,
    }


def require_host_preflight(
    snapshot: dict[str, Any], *, allow_conflicting_processes: bool = False
) -> None:
    if snapshot.get("powerSource") != "AC Power":
        raise ValueError("benchmark host must be on AC power")
    if snapshot.get("lowPowerMode") is not False:
        raise ValueError("benchmark host must have Low Power Mode disabled")
    if snapshot.get("thermalState") != "nominal":
        raise ValueError("benchmark host has non-nominal thermal state")
    conflicting = snapshot.get("conflictingProcesses") or []
    if conflicting and not allow_conflicting_processes:
        raise ValueError(
            "benchmark host has a conflicting benchmark process: "
            + ", ".join(str(process) for process in conflicting)
        )


def acquire_run_lock(cache_root: Path) -> TextIO:
    lock_path = cache_root / "fleet-run.lock"
    lock = lock_path.open("a+", encoding="utf-8")
    try:
        fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError as error:
        lock.close()
        raise ValueError(
            f"another benchmark comparison is using cache root {cache_root}"
        ) from error
    return lock


def wait_for_stable_host(
    *,
    timeout_seconds: float = DEFAULT_IDLE_TIMEOUT_SECONDS,
    interval_seconds: float = DEFAULT_IDLE_INTERVAL_SECONDS,
    max_cpu_percent: float = DEFAULT_IDLE_CPU_PERCENT,
    consecutive_samples: int = DEFAULT_IDLE_SAMPLES,
    snapshot_fn: Callable[[], dict[str, Any]] = host_snapshot,
    sleep_fn: Callable[[float], None] = time.sleep,
) -> dict[str, Any]:
    if timeout_seconds <= 0 or interval_seconds < 0 or consecutive_samples <= 0:
        raise ValueError("idle-settling arguments must be positive")
    deadline = time.monotonic() + timeout_seconds
    idle_samples = 0
    last_snapshot: dict[str, Any] | None = None
    while time.monotonic() <= deadline:
        last_snapshot = snapshot_fn()
        require_host_preflight(last_snapshot, allow_conflicting_processes=True)
        cpu_percent = last_snapshot.get("normalizedCpuPercent")
        conflicts = last_snapshot.get("conflictingProcesses") or []
        if (
            not conflicts
            and isinstance(cpu_percent, (int, float))
            and cpu_percent <= max_cpu_percent
        ):
            idle_samples += 1
            if idle_samples >= consecutive_samples:
                return last_snapshot
        else:
            idle_samples = 0
        sleep_fn(interval_seconds)
    observed = None if last_snapshot is None else last_snapshot.get("normalizedCpuPercent")
    conflicts = (
        [] if last_snapshot is None else last_snapshot.get("conflictingProcesses") or []
    )
    raise ValueError(
        "benchmark host did not become idle within "
        f"{timeout_seconds:.0f}s (last normalized CPU: {observed}; "
        f"conflicting processes: {conflicts})"
    )


def seed_external_binaries(source: Path, worktree: Path) -> None:
    destination = worktree / "app" / "src-tauri" / "binaries"
    destination.mkdir(parents=True, exist_ok=True)
    for name in EXTERNAL_BINARIES:
        source_file = source / name
        if not source_file.is_file():
            raise ValueError(f"required local helper is missing: {source_file}")
        target = destination / name
        if target.exists():
            raise ValueError(f"temporary worktree unexpectedly contains helper: {target}")
        # These helpers are gitignored local build prerequisites. Copying the
        # four exact files keeps detached refs self-contained without exposing
        # or mutating the source checkout's helper directory.
        shutil.copy2(source_file, target)


def run_benchmark(
    *,
    runner: Path,
    worktree: Path,
    output: Path,
    corpus_dir: Path,
    preset: str,
    models: str | None,
    machine_label: str,
    environment: dict[str, str],
) -> None:
    command = [
        sys.executable,
        str(runner),
        "run",
        "--repo",
        str(worktree),
        "--output",
        str(output),
        "--corpus",
        "personal",
        "--corpus-dir",
        str(corpus_dir),
        "--preset",
        preset,
        "--machine-label",
        machine_label,
    ]
    if models:
        command.extend(["--models", models])
    print("+", " ".join(command), flush=True)
    process = subprocess.Popen(command, env=environment, start_new_session=True)
    try:
        while True:
            detected_interference = conflicting_processes(
                process_table(), allowed_root_pid=process.pid
            )
            if detected_interference:
                raise ValueError(
                    "unrelated build process ran during benchmark measurement: "
                    + ", ".join(detected_interference)
                )
            return_code = process.poll()
            if return_code is not None:
                break
            time.sleep(PROCESS_MONITOR_INTERVAL_SECONDS)
    except BaseException:
        terminate_process_group(process)
        raise
    if return_code != 0:
        raise subprocess.CalledProcessError(return_code, command)


def annotate_metadata(report: Path, values: dict[str, Any]) -> None:
    meta = report.with_suffix(".meta.json")
    with meta.open("r", encoding="utf-8") as handle:
        metadata = json.load(handle)
    if not isinstance(metadata, dict):
        raise ValueError(f"benchmark runner wrote invalid metadata: {meta}")
    metadata.update(values)
    meta.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")


def load_cache_summary(report: Path) -> dict[str, Any]:
    with report.open("r", encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict) or not isinstance(value.get("results"), list):
        raise ValueError(f"benchmark runner wrote an invalid report: {report}")
    configuration = value.get("configuration")
    if not isinstance(configuration, dict):
        raise ValueError(f"benchmark report lacks configuration: {report}")
    model_order = configuration.get("modelRunOrder")
    shared_init_order = configuration.get("sharedInitOrder")
    shared_init_ms = value.get("sharedInitMs")
    if (
        not isinstance(model_order, list)
        or not model_order
        or not all(isinstance(model, str) and model for model in model_order)
        or not isinstance(shared_init_order, list)
        or not shared_init_order
        or not all(isinstance(model, str) and model for model in shared_init_order)
        or not isinstance(shared_init_ms, (int, float))
        or isinstance(shared_init_ms, bool)
        or not math.isfinite(shared_init_ms)
        or shared_init_ms <= 0
    ):
        raise ValueError(f"benchmark report lacks cache-boundary data: {report}")
    if not set(shared_init_order).issubset(model_order):
        raise ValueError(
            f"shared initialization targets are not selected models: {report}"
        )
    return {
        "modelRunOrder": model_order,
        "sharedInitOrder": shared_init_order,
        "sharedInitMs": shared_init_ms,
}


def run_conditioned_benchmark(
    *,
    runner: Path,
    worktree: Path,
    report: Path,
    cache_root: Path,
    corpus_dir: Path,
    preset: str,
    conditioning_preset: str,
    models: str | None,
    machine_label: str,
    environment: dict[str, str],
    role: str,
    ref: str,
    sha: str,
    order_position: int,
    idle_timeout_seconds: float,
    idle_interval_seconds: float,
    idle_cpu_percent: float,
    idle_samples: int,
) -> None:
    initial_snapshot = wait_for_stable_host(
        timeout_seconds=idle_timeout_seconds,
        interval_seconds=idle_interval_seconds,
        max_cpu_percent=idle_cpu_percent,
        consecutive_samples=idle_samples,
    )
    with tempfile.TemporaryDirectory(prefix=f"conditioning-{role}-", dir=cache_root) as temp:
        conditioning_report = Path(temp) / "conditioning.json"
        run_benchmark(
            runner=runner,
            worktree=worktree,
            output=conditioning_report,
            corpus_dir=corpus_dir,
            preset=conditioning_preset,
            models=models,
            machine_label=machine_label,
            environment=environment,
        )
        conditioning_meta = conditioning_report.with_suffix(".meta.json")
        with conditioning_meta.open("r", encoding="utf-8") as handle:
            conditioning_metadata = json.load(handle)
        conditioning_summary = load_cache_summary(conditioning_report)

        # The full-set pass ends with the largest Whisper model. Follow it with
        # two identical passes over the benchmark's declared shared-init
        # targets so Parakeet/Core ML inference caches see the same repeated
        # workload immediately before either measured ref.
        tail_models = ",".join(conditioning_summary["sharedInitOrder"])
        tail_metadata: list[dict[str, Any]] = []
        tail_summaries: list[dict[str, Any]] = []
        for probe in range(1, TARGET_CONDITIONING_PASSES + 1):
            tail_report = Path(temp) / f"conditioning-targets-{probe}.json"
            run_benchmark(
                runner=runner,
                worktree=worktree,
                output=tail_report,
                corpus_dir=corpus_dir,
                preset=conditioning_preset,
                models=tail_models,
                machine_label=machine_label,
                environment=environment,
            )
            with tail_report.with_suffix(".meta.json").open(
                "r", encoding="utf-8"
            ) as handle:
                current_metadata = json.load(handle)
            current_summary = load_cache_summary(tail_report)
            if (
                current_summary["modelRunOrder"]
                != conditioning_summary["sharedInitOrder"]
                or current_summary["sharedInitOrder"]
                != conditioning_summary["sharedInitOrder"]
            ):
                raise ValueError("target conditioning changed the shared-init order")
            tail_metadata.append(current_metadata)
            tail_summaries.append(current_summary)

        settled_snapshot = wait_for_stable_host(
            timeout_seconds=idle_timeout_seconds,
            interval_seconds=idle_interval_seconds,
            max_cpu_percent=idle_cpu_percent,
            consecutive_samples=idle_samples,
        )
        run_benchmark(
            runner=runner,
            worktree=worktree,
            output=report,
            corpus_dir=corpus_dir,
            preset=preset,
            models=models,
            machine_label=machine_label,
            environment=environment,
        )
        measured_summary = load_cache_summary(report)
        if measured_summary["modelRunOrder"] != conditioning_summary["modelRunOrder"]:
            raise ValueError("conditioning and measurement model order differ")
        if (
            measured_summary["sharedInitOrder"]
            != conditioning_summary["sharedInitOrder"]
        ):
            raise ValueError("conditioning and measurement shared-init order differ")

    final_snapshot = host_snapshot()
    require_host_preflight(final_snapshot)
    annotate_metadata(
        report,
        {
            "cachePolicy": CACHE_POLICY,
            "conditioningPreset": conditioning_preset,
            "conditioningStages": list(CONDITIONING_STAGES),
            "conditioningGitCommit": sha,
            "conditioningDurationSeconds": round(
                float(conditioning_metadata.get("durationSeconds", 0.0))
                + sum(
                    float(metadata.get("durationSeconds", 0.0))
                    for metadata in tail_metadata
                ),
                3,
            ),
            "fullConditioningDurationSeconds": conditioning_metadata.get(
                "durationSeconds"
            ),
            "fullConditioningSharedInitMs": conditioning_summary["sharedInitMs"],
            "targetConditioningDurationSeconds": [
                metadata.get("durationSeconds") for metadata in tail_metadata
            ],
            "targetConditioningSharedInitMs": [
                summary["sharedInitMs"] for summary in tail_summaries
            ],
            "targetConditioningModelOrder": tail_summaries[-1]["modelRunOrder"],
            "powerSource": settled_snapshot.get("powerSource"),
            "lowPowerMode": settled_snapshot.get("lowPowerMode"),
            "thermalState": settled_snapshot.get("thermalState"),
            "hostBeforeConditioning": initial_snapshot,
            "hostBeforeMeasurement": settled_snapshot,
            "hostAfterMeasurement": final_snapshot,
            "runnerRole": role,
            "runnerRef": ref,
            "runnerOrderPosition": order_position,
            "idleCpuLimitPercent": idle_cpu_percent,
            "idleConsecutiveSamples": idle_samples,
            "idleSampleIntervalSeconds": idle_interval_seconds,
        },
    )


def run_ref(
    *,
    repo: Path,
    ref: str,
    sha: str,
    role: str,
    cache_root: Path,
    report_root: Path,
    corpus_dir: Path,
    preset: str,
    models: str | None,
    machine_label: str,
    stamp: str,
    runner: Path,
    binary_source: Path,
    order_position: int,
    idle_timeout_seconds: float,
    idle_interval_seconds: float,
    idle_cpu_percent: float,
    idle_samples: int,
) -> Path:
    worktree_parent = cache_root / "worktrees"
    worktree_parent.mkdir(parents=True, exist_ok=True)
    worktree = Path(tempfile.mkdtemp(prefix=f"{role}-", dir=worktree_parent))
    # git worktree add requires the destination not to exist.
    worktree.rmdir()
    run(["git", "worktree", "add", "--detach", str(worktree), sha], cwd=repo)
    try:
        seed_external_binaries(binary_source, worktree)
        environment = benchmark_environment(
            cache_root, worktree / "app" / "src-tauri" / "Cargo.lock"
        )
        report = report_root / f"{stamp}-{role}-{safe_label(ref)}-{sha[:12]}.json"
        run_conditioned_benchmark(
            runner=runner,
            worktree=worktree,
            report=report,
            cache_root=cache_root,
            corpus_dir=corpus_dir,
            preset=preset,
            conditioning_preset=CONDITIONING_PRESET,
            models=models,
            machine_label=machine_label,
            environment=environment,
            role=role,
            ref=ref,
            sha=sha,
            order_position=order_position,
            idle_timeout_seconds=idle_timeout_seconds,
            idle_interval_seconds=idle_interval_seconds,
            idle_cpu_percent=idle_cpu_percent,
            idle_samples=idle_samples,
        )
    finally:
        run(["git", "worktree", "remove", "--force", str(worktree)], cwd=repo)
    return report


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True, help="clean source Git repository")
    parser.add_argument("--baseline", required=True, help="baseline Git ref")
    parser.add_argument("--candidate", required=True, help="candidate Git ref")
    parser.add_argument("--corpus-dir", type=Path, required=True)
    parser.add_argument("--cache-root", type=Path, required=True)
    parser.add_argument("--report-root", type=Path, required=True)
    parser.add_argument(
        "--binary-source",
        type=Path,
        help="directory containing the four gitignored aarch64 helper binaries",
    )
    parser.add_argument("--preset", choices=("quick", "standard", "thorough"), default="standard")
    parser.add_argument("--models", help="comma-separated model IDs")
    parser.add_argument("--machine-label", default="fleet-mac")
    parser.add_argument("--candidate-first", action="store_true")
    parser.add_argument("--no-fail", action="store_true")
    parser.add_argument(
        "--idle-timeout-seconds", type=float, default=DEFAULT_IDLE_TIMEOUT_SECONDS
    )
    parser.add_argument(
        "--idle-interval-seconds", type=float, default=DEFAULT_IDLE_INTERVAL_SECONDS
    )
    parser.add_argument(
        "--idle-cpu-percent", type=float, default=DEFAULT_IDLE_CPU_PERCENT
    )
    parser.add_argument("--idle-samples", type=int, default=DEFAULT_IDLE_SAMPLES)
    args = parser.parse_args()

    repo = args.repo.expanduser().resolve()
    corpus_dir = args.corpus_dir.expanduser().resolve()
    cache_root = args.cache_root.expanduser().resolve()
    report_root = args.report_root.expanduser().resolve()
    binary_source = (
        args.binary_source.expanduser().resolve()
        if args.binary_source
        else repo / "app" / "src-tauri" / "binaries"
    )
    if not (repo / ".git").exists():
        parser.error(f"--repo is not a Git repository: {repo}")
    if not (corpus_dir / "manifest.json").is_file():
        parser.error(f"--corpus-dir does not contain manifest.json: {corpus_dir}")
    cache_root.mkdir(parents=True, exist_ok=True)
    report_root.mkdir(parents=True, exist_ok=True)
    run_lock = acquire_run_lock(cache_root)
    wait_for_stable_host(
        timeout_seconds=args.idle_timeout_seconds,
        interval_seconds=args.idle_interval_seconds,
        max_cpu_percent=args.idle_cpu_percent,
        consecutive_samples=args.idle_samples,
    )

    run(["git", "fetch", "--prune", "origin"], cwd=repo)
    baseline_sha = resolve_ref(repo, args.baseline)
    candidate_sha = resolve_ref(repo, args.candidate)
    if baseline_sha == candidate_sha:
        parser.error("baseline and candidate resolve to the same commit")

    runner = Path(__file__).resolve().with_name("murmur_bench.py")
    if not runner.is_file():
        parser.error(f"murmur_bench.py must be installed beside this helper: {runner}")
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    roles = [
        ("baseline", args.baseline, baseline_sha),
        ("candidate", args.candidate, candidate_sha),
    ]
    if args.candidate_first:
        roles.reverse()

    reports: dict[str, Path] = {}
    for order_position, (role, ref, sha) in enumerate(roles, start=1):
        reports[role] = run_ref(
            repo=repo,
            ref=ref,
            sha=sha,
            role=role,
            cache_root=cache_root,
            report_root=report_root,
            corpus_dir=corpus_dir,
            preset=args.preset,
            models=args.models,
            machine_label=args.machine_label,
            stamp=stamp,
            runner=runner,
            binary_source=binary_source,
            order_position=order_position,
            idle_timeout_seconds=args.idle_timeout_seconds,
            idle_interval_seconds=args.idle_interval_seconds,
            idle_cpu_percent=args.idle_cpu_percent,
            idle_samples=args.idle_samples,
        )

    comparison = report_root / f"{stamp}-comparison.json"
    compare_command = [
        sys.executable,
        str(runner),
        "compare",
        str(reports["baseline"]),
        str(reports["candidate"]),
        "--output",
        str(comparison),
    ]
    if args.no_fail:
        compare_command.append("--no-fail")
    run(compare_command)
    print(f"Fleet comparison complete: {comparison}")
    run_lock.close()
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as error:
        raise SystemExit(error.returncode)
    except (OSError, ValueError, json.JSONDecodeError) as error:
        print(f"murmur-bench-remote: {error}", file=sys.stderr)
        raise SystemExit(1)
