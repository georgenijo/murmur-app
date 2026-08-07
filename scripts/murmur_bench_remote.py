#!/usr/bin/env python3
"""Run baseline and candidate Murmur refs on one trusted Fleet Mac.

This helper is intended to be copied to the benchmark Mac and invoked there.
It creates detached temporary worktrees, shares a release Cargo target cache,
runs both refs against the same private corpus, writes local reports, compares
them, and removes only the temporary worktrees it created.
"""

from __future__ import annotations

import argparse
import datetime as dt
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile


def run(command: list[str], *, cwd: Path | None = None, env: dict[str, str] | None = None) -> None:
    print("+", " ".join(command), flush=True)
    subprocess.run(command, cwd=cwd, env=env, check=True)


def output(command: list[str], *, cwd: Path) -> str:
    return subprocess.run(
        command, cwd=cwd, check=True, capture_output=True, text=True
    ).stdout.strip()


def safe_label(value: str) -> str:
    label = re.sub(r"[^A-Za-z0-9._-]+", "-", value).strip("-.")
    return (label or "ref")[:80]


def resolve_ref(repo: Path, ref: str) -> str:
    return output(["git", "rev-parse", "--verify", f"{ref}^{{commit}}"], cwd=repo)


def benchmark_environment(cache_root: Path) -> dict[str, str]:
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
    environment["CARGO_TARGET_DIR"] = str(cache_root / "cargo-target")
    return environment


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
    environment: dict[str, str],
    runner: Path,
) -> Path:
    worktree_parent = cache_root / "worktrees"
    worktree_parent.mkdir(parents=True, exist_ok=True)
    worktree = Path(tempfile.mkdtemp(prefix=f"{role}-", dir=worktree_parent))
    # git worktree add requires the destination not to exist.
    worktree.rmdir()
    run(["git", "worktree", "add", "--detach", str(worktree), sha], cwd=repo)
    report = report_root / f"{stamp}-{role}-{safe_label(ref)}-{sha[:12]}.json"
    command = [
        sys.executable,
        str(runner),
        "run",
        "--repo",
        str(worktree),
        "--output",
        str(report),
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
    try:
        run(command, env=environment)
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
    parser.add_argument("--preset", choices=("quick", "standard", "thorough"), default="standard")
    parser.add_argument("--models", help="comma-separated model IDs")
    parser.add_argument("--machine-label", default="fleet-mac")
    parser.add_argument("--candidate-first", action="store_true")
    parser.add_argument("--no-fail", action="store_true")
    args = parser.parse_args()

    repo = args.repo.expanduser().resolve()
    corpus_dir = args.corpus_dir.expanduser().resolve()
    cache_root = args.cache_root.expanduser().resolve()
    report_root = args.report_root.expanduser().resolve()
    if not (repo / ".git").exists():
        parser.error(f"--repo is not a Git repository: {repo}")
    if not (corpus_dir / "manifest.json").is_file():
        parser.error(f"--corpus-dir does not contain manifest.json: {corpus_dir}")
    cache_root.mkdir(parents=True, exist_ok=True)
    report_root.mkdir(parents=True, exist_ok=True)

    run(["git", "fetch", "--prune", "origin"], cwd=repo)
    baseline_sha = resolve_ref(repo, args.baseline)
    candidate_sha = resolve_ref(repo, args.candidate)
    if baseline_sha == candidate_sha:
        parser.error("baseline and candidate resolve to the same commit")

    environment = benchmark_environment(cache_root)
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
    for role, ref, sha in roles:
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
            environment=environment,
            runner=runner,
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
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except subprocess.CalledProcessError as error:
        raise SystemExit(error.returncode)
