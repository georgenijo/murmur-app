#!/usr/bin/env python3
"""Re-run the C13.3 revert and mutation rows against a checkout.

Each row in `scripts/c13_3_mutations/*.json` is one source edit that reverts a
historical fix, or mutates the named C3 row, `admit` check or executor effect, plus
the cargo commands that must then fail. A row is CAUGHT when a command fails with a
test failure, SURVIVED when every command passes, and INVALID when a command
stopped on a compile error (a mutation that does not build proves nothing).

    python3 scripts/c13_3_mutations.py --list
    python3 scripts/c13_3_mutations.py --family stop
    python3 scripts/c13_3_mutations.py --row M216-stop-queued-behind-start --root ../scratch-worktree
    python3 scripts/c13_3_mutations.py --all --check        # edits still apply; no cargo

Run it in a scratch worktree: it edits tracked source in place and restores each
file from its original bytes afterwards (also on Ctrl-C). It refuses to touch a file
with uncommitted changes. See docs/testing/c13-3-mutations.md.

Exit status: 0 when every selected row was CAUGHT (or `--check`/`--list`/`--dry-run`
succeeded), 1 when a row SURVIVED or was INVALID, 2 for a usage or spec error.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
SPEC_DIR = SCRIPT_DIR / "c13_3_mutations"
REPO_ROOT = SCRIPT_DIR.parent

# The six decision families of the dictation core. A row belongs to exactly one.
FAMILIES = ("admission", "cancel", "lifecycle", "process_audio", "start", "stop")

CARGO_DIR = "app/src-tauri"
# `error: test failed, to rerun pass ...` is a test failure; only a compile stop is INVALID.
BUILD_ERROR = re.compile(r"^error\[E\d+\]|could not compile|^error: aborting due to", re.MULTILINE)


class SpecError(ValueError):
    """A row file that does not have the shape the runner needs."""


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="c13_3_mutations.py",
        description="Re-run the C13.3 revert and mutation rows (see the module docstring).",
    )
    parser.add_argument(
        "--family",
        action="append",
        choices=FAMILIES,
        default=[],
        help="run only this family's rows (repeatable)",
    )
    parser.add_argument(
        "--row",
        action="append",
        default=[],
        metavar="NAME",
        help="run only the named row (repeatable)",
    )
    parser.add_argument("--all", action="store_true", help="run every row")
    parser.add_argument("--list", action="store_true", help="list the selected rows and exit")
    parser.add_argument(
        "--check",
        action="store_true",
        help="verify each edit still applies exactly once; run no cargo command",
    )
    parser.add_argument(
        "--dry-run",
        action="store_true",
        help="print the edits and commands that would run; change nothing",
    )
    parser.add_argument(
        "--root",
        type=Path,
        default=REPO_ROOT,
        help="the checkout to mutate (default: this repository)",
    )
    parser.add_argument(
        "--keep-going",
        action="store_true",
        help="after a SURVIVED or INVALID row, run the remaining rows",
    )
    return parser


def parse_args(argv: list[str]) -> argparse.Namespace:
    """Parse and validate the command line; a bad combination exits with status 2."""
    parser = build_parser()
    args = parser.parse_args(argv)
    selected = bool(args.family or args.row or args.all or args.list)
    if not selected:
        parser.error("choose rows with --family, --row or --all (or --list)")
    if args.all and (args.family or args.row):
        parser.error("--all cannot be combined with --family or --row")
    if args.check and args.dry_run:
        parser.error("--check and --dry-run are separate modes")
    return args


def load_rows(spec_dir: Path = SPEC_DIR) -> list[dict]:
    rows = []
    for path in sorted(spec_dir.glob("*.json")):
        row = json.loads(path.read_text())
        validate_row(row, path)
        rows.append(row)
    names = [row["name"] for row in rows]
    duplicates = {name for name in names if names.count(name) > 1}
    if duplicates:
        raise SpecError(f"duplicate row names: {sorted(duplicates)}")
    return rows


def validate_row(row: dict, path: Path) -> None:
    where = path.name
    for key in ("name", "family", "bug", "description", "edits", "cmds"):
        if key not in row:
            raise SpecError(f"{where}: missing `{key}`")
    if row["name"] != path.stem:
        raise SpecError(f"{where}: `name` must equal the file name")
    if row["family"] not in FAMILIES:
        raise SpecError(f"{where}: unknown family {row['family']!r}")
    if not row["edits"] or not row["cmds"]:
        raise SpecError(f"{where}: a row needs at least one edit and one command")
    for edit in row["edits"]:
        if not (isinstance(edit, list) and len(edit) == 3 and all(isinstance(x, str) for x in edit)):
            raise SpecError(f"{where}: an edit is [file, old, new]")
        if edit[1] == edit[2]:
            raise SpecError(f"{where}: an edit that changes nothing")
    for cmd in row["cmds"]:
        if not (isinstance(cmd, list) and cmd and all(isinstance(x, str) for x in cmd)):
            raise SpecError(f"{where}: a command is a non-empty list of strings")


def select_rows(rows: list[dict], args: argparse.Namespace) -> list[dict]:
    if args.all or args.list and not (args.family or args.row):
        chosen = list(rows)
    else:
        chosen = [
            row
            for row in rows
            if row["family"] in args.family or row["name"] in args.row
        ]
    unknown = sorted(set(args.row) - {row["name"] for row in rows})
    if unknown:
        raise SpecError(f"unknown row(s): {unknown}")
    return chosen


def git_dirty(root: Path, path: Path) -> bool:
    result = subprocess.run(
        ["git", "status", "--porcelain", "--", str(path)],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    )
    return bool(result.stdout.strip())


def plan_edits(row: dict, root: Path) -> list[tuple[Path, str, str, str]]:
    """Resolve each edit to (path, original text, old, new); one occurrence each."""
    crate = root / CARGO_DIR
    planned = []
    for file, old, new in row["edits"]:
        path = crate / file
        text = path.read_text()
        count = text.count(old)
        if count != 1:
            raise SpecError(f"{row['name']}: `{old[:60]!r}` occurs {count}x in {file}")
        planned.append((path, text, old, new))
    return planned


def run_row(row: dict, root: Path, check_only: bool, dry_run: bool) -> str:
    """Run one row and return CAUGHT, SURVIVED, INVALID, APPLIES or DRY."""
    planned = plan_edits(row, root)
    if check_only:
        return "APPLIES"
    if dry_run:
        for path, _, old, new in planned:
            print(f"  edit {path.relative_to(root)}: {old[:70]!r} -> {new[:70]!r}")
        for cmd in row["cmds"]:
            print("  run  " + " ".join(cmd))
        return "DRY"
    for path, _, _, _ in planned:
        if git_dirty(root, path):
            raise SpecError(f"{path} has uncommitted changes; use a clean scratch worktree")
    originals: dict[Path, str] = {}
    env = dict(os.environ, CARGO_BUILD_JOBS="4")
    try:
        for path, text, old, new in planned:
            originals.setdefault(path, text)
            path.write_text(path.read_text().replace(old, new))
        verdict = "SURVIVED"
        for cmd in row["cmds"]:
            result = subprocess.run(
                cmd, cwd=root / CARGO_DIR, env=env, capture_output=True, text=True
            )
            output = result.stdout + result.stderr
            print(f"  $ {' '.join(cmd)} -> exit {result.returncode}")
            if result.returncode != 0:
                for line in output.splitlines():
                    if re.search(r"^test .* FAILED|^error(\[|:)|panicked at", line):
                        print("   ", line)
                verdict = "INVALID" if BUILD_ERROR.search(output) else "CAUGHT"
                break
        return verdict
    finally:
        for path, text in originals.items():
            path.write_text(text)


def main(argv: list[str]) -> int:
    args = parse_args(argv)
    try:
        rows = load_rows()
        chosen = select_rows(rows, args)
    except SpecError as error:
        print(f"error: {error}", file=sys.stderr)
        return 2
    if args.list:
        for row in chosen:
            print(f"{row['family']:<14}{row['name']:<42}{row['bug']}")
        return 0
    if not chosen:
        print("no C13.3 rows in the selected family (the cancel family has none)")
        return 0
    failed = False
    for row in chosen:
        print(f"== {row['name']} [{row['family']}] {row['bug']}: {row['description']}")
        try:
            verdict = run_row(row, args.root.resolve(), args.check, args.dry_run)
        except SpecError as error:
            print(f"error: {error}", file=sys.stderr)
            return 2
        print(f"== {row['name']}: {verdict}")
        if verdict in ("SURVIVED", "INVALID"):
            failed = True
            if not args.keep_going:
                break
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
