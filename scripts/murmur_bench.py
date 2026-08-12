#!/usr/bin/env python3
"""Run and compare Murmur's private, repeatable on-device benchmarks.

Reports and their metadata stay outside the repository by default. The runner
does not download models or upload audio/results; it only invokes the existing
release-mode Rust benchmark harness against already-installed local assets.
"""

from __future__ import annotations

import argparse
import datetime as dt
import json
import math
import os
from pathlib import Path
import subprocess
import sys
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_REPORT_ROOT = (
    Path.home() / "Library" / "Application Support" / "Murmur Bench" / "reports"
)
LATENCY_RELATIVE_LIMIT = 0.10
LATENCY_ABSOLUTE_LIMIT_MS = 25.0
WER_ABSOLUTE_LIMIT = 0.01
MEMORY_ABSOLUTE_LIMIT_MB = 128.0
CONDITIONED_CACHE_POLICY = "conditioned-timed-path-v2"
CONDITIONING_STAGES = (
    "all-selected-quick",
    "shared-init-targets-quick-x2",
)
TARGET_CONDITIONING_PASSES = 2


def git(repo: Path, *args: str) -> str:
    result = subprocess.run(
        ["git", *args], cwd=repo, check=True, capture_output=True, text=True
    )
    return result.stdout.strip()


def default_output(repo: Path) -> Path:
    stamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    commit = git(repo, "rev-parse", "--short=12", "HEAD")
    return DEFAULT_REPORT_ROOT / f"{stamp}-{commit}.json"


def metadata_path(report: Path) -> Path:
    return report.with_suffix(".meta.json")


def run_benchmark(args: argparse.Namespace) -> int:
    repo = args.repo.resolve()
    output = (args.output or default_output(repo)).expanduser().resolve()
    output.parent.mkdir(parents=True, exist_ok=True)

    command = ["cargo", "test", "--release"]
    if args.corpus == "personal":
        command.extend(["--features", "internal-benchmark"])
    command.extend(
        [
            "--test",
            "headless_benchmark",
            "headless_benchmark",
            "--",
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ]
    )

    environment = os.environ.copy()
    environment["MURMUR_BENCH_OUT"] = str(output)
    environment["MURMUR_BENCH_PRESET"] = args.preset
    environment["MURMUR_BENCH_CORPUS"] = args.corpus
    if args.models:
        environment["MURMUR_BENCH_MODELS"] = args.models
    if args.corpus_dir:
        corpus_dir = args.corpus_dir.expanduser().resolve()
        if not corpus_dir.is_absolute():
            raise ValueError("--corpus-dir must resolve to an absolute path")
        environment["MURMUR_BENCH_CORPUS_DIR"] = str(corpus_dir)

    started_at = dt.datetime.now(dt.timezone.utc)
    print(f"Running {args.corpus} {args.preset} benchmark -> {output}", flush=True)
    subprocess.run(command, cwd=repo / "app" / "src-tauri", env=environment, check=True)
    finished_at = dt.datetime.now(dt.timezone.utc)

    # Parse before publishing metadata so an interrupted/invalid run never
    # looks complete to the Fleet collector.
    with output.open("r", encoding="utf-8") as handle:
        report = json.load(handle)
    if not isinstance(report, dict) or not isinstance(report.get("results"), list):
        raise ValueError("benchmark harness wrote an invalid report")

    dirty = bool(git(repo, "status", "--porcelain"))
    metadata = {
        "schemaVersion": 1,
        "reportFile": output.name,
        "gitCommit": git(repo, "rev-parse", "HEAD"),
        "gitBranch": git(repo, "branch", "--show-current"),
        "gitDirty": dirty,
        "machineLabel": args.machine_label,
        "corpus": args.corpus,
        "preset": args.preset,
        "models": args.models or "all-installed",
        "startedAt": started_at.isoformat(),
        "finishedAt": finished_at.isoformat(),
        "durationSeconds": round((finished_at - started_at).total_seconds(), 3),
    }
    meta = metadata_path(output)
    meta.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    print(f"Report:   {output}")
    print(f"Metadata: {meta}")
    return 0


def load_report(path: Path) -> dict[str, Any]:
    with path.expanduser().resolve().open("r", encoding="utf-8") as handle:
        report = json.load(handle)
    if not isinstance(report, dict) or not isinstance(report.get("results"), list):
        raise ValueError(f"{path} is not a Murmur benchmark report")
    return report


def load_optional_metadata(report: Path) -> dict[str, Any] | None:
    meta = metadata_path(report)
    if not meta.is_file():
        return None
    with meta.open("r", encoding="utf-8") as handle:
        value = json.load(handle)
    if not isinstance(value, dict):
        raise ValueError(f"{meta} is not Murmur benchmark metadata")
    return value


def comparable_identity(report: dict[str, Any]) -> tuple[Any, ...]:
    environment = report.get("environment") or {}
    corpus = report.get("corpus") or {}
    configuration = report.get("configuration") or {}
    return (
        environment.get("os"),
        environment.get("osVersion"),
        environment.get("architecture"),
        environment.get("hardwareModel"),
        environment.get("chip"),
        corpus.get("source"),
        tuple(corpus.get("fixtureIds") or []),
        report.get("preset"),
        report.get("iterations"),
        configuration.get("vadThreshold"),
        configuration.get("executionPath"),
        configuration.get("transcriptTransformProfile"),
        configuration.get("percentileMethod"),
        tuple(configuration.get("modelRunOrder") or []),
        tuple(configuration.get("sharedInitOrder") or []),
    )


def differs_beyond_latency_tolerance(
    left: float,
    right: float,
    *,
    relative_limit: float = LATENCY_RELATIVE_LIMIT,
    absolute_limit_ms: float = LATENCY_ABSOLUTE_LIMIT_MS,
) -> bool:
    smaller = min(left, right)
    absolute = abs(right - left)
    relative = absolute / smaller if smaller > 0 else None
    return relative is None or (
        relative > relative_limit and absolute > absolute_limit_ms
    )


def validate_comparability(
    baseline: dict[str, Any],
    candidate: dict[str, Any],
    baseline_metadata: dict[str, Any] | None,
    candidate_metadata: dict[str, Any] | None,
) -> None:
    baseline_configuration = baseline.get("configuration") or {}
    candidate_configuration = candidate.get("configuration") or {}
    baseline_models = tuple(baseline_configuration.get("modelRunOrder") or [])
    candidate_models = tuple(candidate_configuration.get("modelRunOrder") or [])
    if baseline_models != candidate_models:
        raise ValueError("reports are not comparable: model set or run order differs")
    if comparable_identity(baseline) != comparable_identity(candidate):
        raise ValueError(
            "reports are not comparable: hardware, OS, corpus, preset, iterations, "
            "VAD threshold, execution path, transform profile, percentile method, "
            "or shared initialization order differ"
        )

    baseline_policy = (baseline_metadata or {}).get("cachePolicy")
    candidate_policy = (candidate_metadata or {}).get("cachePolicy")
    if baseline_policy != candidate_policy:
        raise ValueError("reports are not comparable: cache conditioning policy differs")
    if baseline_policy is None:
        # Preserve local comparison support for reports created outside the
        # release runner. Fleet release reports always carry an explicit policy.
        return
    if baseline_policy != CONDITIONED_CACHE_POLICY:
        raise ValueError(
            f"reports are not comparable: unsupported cache conditioning policy {baseline_policy!r}"
        )

    conditioning_identity: tuple[Any, ...] | None = None
    for metadata, role in (
        (baseline_metadata or {}, "baseline"),
        (candidate_metadata or {}, "candidate"),
    ):
        if metadata.get("conditioningPreset") != "quick":
            raise ValueError(
                f"reports are not comparable: {role} conditioning preset is not quick"
            )
        if tuple(metadata.get("conditioningStages") or ()) != CONDITIONING_STAGES:
            raise ValueError(
                f"reports are not comparable: {role} conditioning stages differ"
            )
        if metadata.get("powerSource") != "AC Power":
            raise ValueError(f"reports are not comparable: {role} did not run on AC power")
        if metadata.get("lowPowerMode") is not False:
            raise ValueError(f"reports are not comparable: {role} used Low Power Mode")
        if metadata.get("thermalState") != "nominal":
            raise ValueError(
                f"reports are not comparable: {role} had non-nominal thermal state"
            )
        if metadata.get("gitDirty") is not False:
            raise ValueError(
                f"reports are not comparable: {role} worktree was not clean"
            )
        if metadata.get("conditioningGitCommit") != metadata.get("gitCommit"):
            raise ValueError(
                f"reports are not comparable: {role} conditioning commit differs "
                "from its measured commit"
            )
        configuration = (
            baseline_configuration if role == "baseline" else candidate_configuration
        )
        if metadata.get("targetConditioningModelOrder") != configuration.get(
            "sharedInitOrder"
        ):
            raise ValueError(
                f"reports are not comparable: {role} target conditioning order differs"
            )
        target_probes = metadata.get("targetConditioningSharedInitMs")
        if (
            not isinstance(target_probes, list)
            or len(target_probes) != TARGET_CONDITIONING_PASSES
            or not all(
                isinstance(value, (int, float))
                and not isinstance(value, bool)
                and math.isfinite(value)
                and value > 0
                for value in target_probes
            )
        ):
            raise ValueError(
                f"reports are not comparable: {role} lacks target conditioning evidence"
            )
        after = metadata.get("hostAfterMeasurement")
        if not isinstance(after, dict):
            raise ValueError(
                f"reports are not comparable: {role} lacks a post-measurement host snapshot"
            )
        if (
            after.get("powerSource") != "AC Power"
            or after.get("lowPowerMode") is not False
            or after.get("thermalState") != "nominal"
        ):
            raise ValueError(
                f"reports are not comparable: {role} host state changed during measurement"
            )

        current_conditioning_identity = (
            metadata.get("conditioningPreset"),
            metadata.get("idleCpuLimitPercent"),
            metadata.get("idleConsecutiveSamples"),
            metadata.get("idleSampleIntervalSeconds"),
        )
        if conditioning_identity is None:
            conditioning_identity = current_conditioning_identity
        elif conditioning_identity != current_conditioning_identity:
            raise ValueError(
                "reports are not comparable: host-settling configuration differs"
            )

    baseline_shared = baseline.get("sharedInitMs")
    candidate_shared = candidate.get("sharedInitMs")
    if (
        not isinstance(baseline_shared, (int, float))
        or isinstance(baseline_shared, bool)
        or not math.isfinite(baseline_shared)
        or baseline_shared <= 0
        or not isinstance(candidate_shared, (int, float))
        or isinstance(candidate_shared, bool)
        or not math.isfinite(candidate_shared)
        or candidate_shared <= 0
    ):
        raise ValueError(
            "reports are not comparable: conditioned reports require sharedInitMs"
        )


def percent_delta(before: float, after: float) -> float | None:
    if before == 0:
        return None
    return (after - before) / before


def fmt_delta(delta: float | None, percentage: bool = False) -> str:
    if delta is None:
        return "n/a"
    return f"{delta * 100:+.1f}%" if percentage else f"{delta:+.3f}"


def compare_reports(args: argparse.Namespace) -> int:
    baseline_path = args.baseline.expanduser().resolve()
    candidate_path = args.candidate.expanduser().resolve()
    baseline = load_report(baseline_path)
    candidate = load_report(candidate_path)
    baseline_metadata = load_optional_metadata(baseline_path)
    candidate_metadata = load_optional_metadata(candidate_path)
    validate_comparability(
        baseline,
        candidate,
        baseline_metadata,
        candidate_metadata,
    )

    baseline_results = {
        item.get("modelName"): item
        for item in baseline["results"]
        if isinstance(item, dict) and item.get("modelName") and not item.get("error")
    }
    candidate_results = {
        item.get("modelName"): item
        for item in candidate["results"]
        if isinstance(item, dict) and item.get("modelName") and not item.get("error")
    }
    common = sorted(set(baseline_results) & set(candidate_results))
    if not common:
        raise ValueError("reports contain no common successful model results")

    baseline_shared = float(baseline["sharedInitMs"])
    candidate_shared = float(candidate["sharedInitMs"])
    shared_init_skew = differs_beyond_latency_tolerance(
        baseline_shared,
        candidate_shared,
        relative_limit=args.latency_limit,
        absolute_limit_ms=args.latency_ms,
    )

    rows: list[dict[str, Any]] = []
    regressions: list[str] = []
    latency_metrics = ("modelLoadMs", "firstInferenceMs", "warmMedianMs", "warmP95Ms", "realtimeFactor")
    accuracy_metrics = ("normalizedWordErrorRate", "deliveredNormalizedWordErrorRate")

    for model in common:
        before = baseline_results[model]
        after = candidate_results[model]
        model_row: dict[str, Any] = {"modelName": model, "metrics": {}}
        for metric in latency_metrics:
            old = before.get(metric)
            new = after.get(metric)
            if not isinstance(old, (int, float)) or not isinstance(new, (int, float)):
                continue
            relative = percent_delta(float(old), float(new))
            absolute = float(new) - float(old)
            # RTF is dimensionless, so only use the relative guard there.
            regressed = (
                relative is not None
                and relative > args.latency_limit
                and (metric == "realtimeFactor" or absolute > args.latency_ms)
            )
            model_row["metrics"][metric] = {
                "baseline": old,
                "candidate": new,
                "absoluteDelta": absolute,
                "relativeDelta": relative,
                "regression": regressed,
            }
            if regressed:
                regressions.append(f"{model} {metric} {fmt_delta(relative, True)}")
        for metric in accuracy_metrics:
            old = before.get(metric)
            new = after.get(metric)
            if not isinstance(old, (int, float)) or not isinstance(new, (int, float)):
                continue
            absolute = float(new) - float(old)
            regressed = absolute > args.wer_limit
            model_row["metrics"][metric] = {
                "baseline": old,
                "candidate": new,
                "absoluteDelta": absolute,
                "relativeDelta": percent_delta(float(old), float(new)),
                "regression": regressed,
            }
            if regressed:
                regressions.append(f"{model} {metric} {absolute * 100:+.1f} points")
        old_memory = before.get("memoryDeltaMb")
        new_memory = after.get("memoryDeltaMb")
        if isinstance(old_memory, (int, float)) and isinstance(new_memory, (int, float)):
            absolute = float(new_memory) - float(old_memory)
            regressed = absolute > args.memory_mb
            model_row["metrics"]["memoryDeltaMb"] = {
                "baseline": old_memory,
                "candidate": new_memory,
                "absoluteDelta": absolute,
                "relativeDelta": percent_delta(float(old_memory), float(new_memory)),
                "regression": regressed,
            }
            if regressed:
                regressions.append(f"{model} memoryDeltaMb {absolute:+.0f} MB")
        rows.append(model_row)

    comparison = {
        "schemaVersion": 1,
        "createdAt": dt.datetime.now(dt.timezone.utc).isoformat(),
        "baseline": str(baseline_path),
        "candidate": str(candidate_path),
        "cachePolicy": (candidate_metadata or {}).get("cachePolicy"),
        "sharedInitMs": {
            "baseline": baseline_shared,
            "candidate": candidate_shared,
            "informationalSkew": shared_init_skew,
        },
        "thresholds": {
            "latencyRelative": args.latency_limit,
            "latencyAbsoluteMs": args.latency_ms,
            "werAbsolute": args.wer_limit,
            "memoryAbsoluteMb": args.memory_mb,
        },
        "regression": bool(regressions),
        "regressions": regressions,
        "models": rows,
    }
    if args.output:
        output = args.output.expanduser().resolve()
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_text(json.dumps(comparison, indent=2) + "\n", encoding="utf-8")
        print(f"Comparison: {output}")

    print(f"Compared {len(common)} model(s) on the same hardware and corpus.")
    if shared_init_skew:
        print(
            "Informational: untimed sharedInitMs differs "
            f"({baseline_shared:.3f} ms vs {candidate_shared:.3f} ms); "
            "per-model timers start after this setup warm-up."
        )
    for row in rows:
        print(f"\n{row['modelName']}")
        for metric, values in row["metrics"].items():
            suffix = "  REGRESSION" if values["regression"] else ""
            relative = fmt_delta(values["relativeDelta"], True)
            print(
                f"  {metric:34} {values['baseline']:>10.3f} -> "
                f"{values['candidate']:>10.3f}  {relative:>8}{suffix}"
            )
    if regressions:
        print("\nRegression gate failed:")
        for regression in regressions:
            print(f"  - {regression}")
        return 0 if args.no_fail else 2
    print("\nNo configured regression threshold was exceeded.")
    return 0


def parser() -> argparse.ArgumentParser:
    root = argparse.ArgumentParser(description=__doc__)
    subcommands = root.add_subparsers(dest="command", required=True)

    run = subcommands.add_parser("run", help="run one release-mode benchmark")
    run.add_argument("--repo", type=Path, default=REPO_ROOT)
    run.add_argument("--output", type=Path)
    run.add_argument("--corpus", choices=("personal", "bundled"), default="personal")
    run.add_argument("--corpus-dir", type=Path)
    run.add_argument("--preset", choices=("quick", "standard", "thorough"), default="standard")
    run.add_argument("--models", help="comma-separated model IDs; default: all installed")
    run.add_argument("--machine-label", default=None)
    run.set_defaults(handler=run_benchmark)

    compare = subcommands.add_parser("compare", help="compare baseline and candidate reports")
    compare.add_argument("baseline", type=Path)
    compare.add_argument("candidate", type=Path)
    compare.add_argument("--output", type=Path)
    compare.add_argument("--latency-limit", type=float, default=LATENCY_RELATIVE_LIMIT)
    compare.add_argument("--latency-ms", type=float, default=LATENCY_ABSOLUTE_LIMIT_MS)
    compare.add_argument("--wer-limit", type=float, default=WER_ABSOLUTE_LIMIT)
    compare.add_argument("--memory-mb", type=float, default=MEMORY_ABSOLUTE_LIMIT_MB)
    compare.add_argument("--no-fail", action="store_true")
    compare.set_defaults(handler=compare_reports)
    return root


def main() -> int:
    args = parser().parse_args()
    try:
        return args.handler(args)
    except (OSError, subprocess.CalledProcessError, ValueError, json.JSONDecodeError) as error:
        print(f"murmur-bench: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
