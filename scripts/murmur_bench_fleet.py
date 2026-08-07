#!/usr/bin/env python3
"""Invoke the private baseline/candidate benchmark on a trusted Fleet Mac."""

from __future__ import annotations

import argparse
import shlex
import subprocess
import sys


DEFAULT_NODE = "mac-mini"
DEFAULT_REPO = "/Users/george-mac-mini/Documents/code/murmur-app"
DEFAULT_TOOL = (
    "/Users/george-mac-mini/Library/Application Support/"
    "Murmur Bench/tools/murmur_bench_remote.py"
)
DEFAULT_CORPUS = (
    "/Users/george-mac-mini/Library/Application Support/"
    "Murmur Benchmark Corpus/v1"
)
DEFAULT_CACHE = "/Users/george-mac-mini/Library/Caches/Murmur Bench"
DEFAULT_REPORTS = (
    "/Users/george-mac-mini/Library/Application Support/Murmur Bench/reports"
)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--candidate", required=True, help="pushed candidate Git ref")
    parser.add_argument("--baseline", default="origin/main", help="baseline Git ref")
    parser.add_argument("--node", default=DEFAULT_NODE)
    parser.add_argument("--preset", choices=("quick", "standard", "thorough"), default="standard")
    parser.add_argument("--models", help="comma-separated model IDs")
    parser.add_argument("--candidate-first", action="store_true")
    parser.add_argument("--no-fail", action="store_true")
    parser.add_argument("--timeout", type=int, default=7200)
    args = parser.parse_args()

    remote = [
        "python3",
        DEFAULT_TOOL,
        "--repo",
        DEFAULT_REPO,
        "--baseline",
        args.baseline,
        "--candidate",
        args.candidate,
        "--corpus-dir",
        DEFAULT_CORPUS,
        "--cache-root",
        DEFAULT_CACHE,
        "--report-root",
        DEFAULT_REPORTS,
        "--preset",
        args.preset,
        "--machine-label",
        args.node,
    ]
    if args.models:
        remote.extend(["--models", args.models])
    if args.candidate_first:
        remote.append("--candidate-first")
    if args.no_fail:
        remote.append("--no-fail")

    command = [
        "fleet",
        "exec",
        "--timeout",
        str(args.timeout),
        args.node,
        "--",
        shlex.join(remote),
    ]
    print("Running benchmark on Fleet node", args.node, flush=True)
    return subprocess.run(command, check=False).returncode


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except OSError as error:
        print(f"murmur-bench-fleet: {error}", file=sys.stderr)
        raise SystemExit(1)
