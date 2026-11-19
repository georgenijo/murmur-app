#!/usr/bin/env python3
"""Fail if the shadow capture changed a production statement (D-S2c-8 a, D-S2c-9 f).

The dictation shadow is **insertion only** in production code: against `main`, a production
file's diff adds lines and removes none. No existing statement is removed, moved or
reordered, so the default build's behaviour (and a feature build's with the shadow
disarmed) is the code `main` already reviewed. `git diff --numstat --no-renames` reports
the removed lines of each file, a renamed file as the deletion of the old path plus the
addition of the new one; this check requires 0 removed lines for **every changed production
file** (any `.rs` under `app/src-tauri/src` or `app/src-tauri/crates/*/src`), except

* test files: a name ending in `_tests.rs` or `tests.rs`, a file that declares
  `#![cfg(test)]`, a file under a `tests/` directory, and the named `TEST_ONLY_FILES`. A
  file is classified from its **base** revision (the branch's text only for a new file), so
  adding `#![cfg(test)]` to an existing production file does not exempt its deletions;
* the named shadow-only modules (`SHADOW_ONLY`): code that exists only for the shadow;
* what `SHADOW_DIFF_ALLOW` names (a path, the most lines it may remove, and why).

    python3 scripts/check_shadow_diff.py [--base origin/main]

It also rejects an always-false gate (`#[cfg(any())]`, `cfg(not(all()))`, `cfg(all(any()))`
and the like) in front of a statement the base already had: that disables an original
statement without deleting a line, which is a removal by another name (D-S2c-10 R8).

An original live statement must never be disabled **in the feature build** either
(D-S2c-8 amendment 1): a gate that is false once `feature = "dictation-shadow"` is on
(`#[cfg(not(feature = "dictation-shadow"))]`, `all(not(feature = ..), ..)` and the like)
in front of a statement the base already had runs a *different* statement in the feature
build. Such a gate is rejected unless the statement is named in `SHADOW_GATED_OFF_ALLOW`
(path, the statement's first line, how many, and why); the only entries meant to be there
are `lib.rs`'s `generate_handler!` entries.

A file's additions are checked by the Rust tests (`shadow_off_by_default` reads every
`cfg` attribute): this check is the other half, that nothing was taken out.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TAURI = "app/src-tauri"

# Code that exists only for the shadow (a prefix names a directory, otherwise a file).
SHADOW_ONLY = [
    f"{TAURI}/src/dictation_shadow/",
    f"{TAURI}/src/dictation_executor/shadow.rs",
    f"{TAURI}/src/commands/recording/shadow_commands.rs",
]

# Test code the name and the file's own attributes do not say is test code: a harness
# included only under `#[cfg(test)]`.
TEST_ONLY_FILES = [
    f"{TAURI}/src/commands/recording_golden_harness.rs",
]

# A production file that may lose lines: `path -> (most removed lines, why)`. An entry needs
# a reason a reviewer can weigh; this list is a reviewed exception, never a convenience.
SHADOW_DIFF_ALLOW: dict[str, tuple[int, str]] = {
    f"{TAURI}/src/main_thread_watch.rs": (
        1,
        "test-only: the `tests` module parses `lib.rs`'s `generate_handler!` list up to the "
        "first `]`; the shadow's `#[cfg(..)]` entries inside the macro carry their own `]`, so "
        "that one line now stops at the macro's closing `]))`. No production statement changes.",
    ),
}

# Existing statements the feature build may switch off, because a feature twin runs in
# their place: `path -> {the statement's first line (stripped): (how many, why)}`. The
# only entries D-S2c-8 amendment 1 allows are the `generate_handler!` entries of `lib.rs`
# (the wrapper command has the same name and calls the original). The rest are the
# statement twins of the earlier rounds, each of which re-runs the same call with the
# flight attached: kept here by name, so a new one (the auto-stop's was one) is rejected,
# and listed so that converting them to the borrowed `ports.shadow_flight()` slot removes
# the entry. A reviewer weighs every line below.
_HANDLER = "the wrapper command of the same name is inserted with the opposite gate and calls this one (D-S2c-8 amendment 1)"
_TWIN = "statement twin (rounds 1-7): the feature build runs the same call with the flight attached; to be converted to the ports slot"
SHADOW_GATED_OFF_ALLOW: dict[str, dict[str, tuple[int, str]]] = {
    f"{TAURI}/src/lib.rs": {
        "commands::recording::process_audio,": (1, _HANDLER),
        "commands::recording::start_native_recording,": (1, _HANDLER),
        "commands::recording::stop_native_recording,": (1, _HANDLER),
        "commands::recording::cancel_native_recording,": (1, _HANDLER),
    },
    f"{TAURI}/src/commands/recording.rs": {
        "fn spawn_dictation_partial_ticker(": (1, _TWIN + "; the feature twin is the one shared ticker lookup"),
        "self.history.ret_pipeline(self.rid);": (1, _TWIN + "; the twin keeps the same retirement and its answer"),
        "let pre = stop_ports::try_run_phase(": (1, _TWIN),
        "let decide = || {": (1, _TWIN),
        "stop_ports::run_phase(": (2, _TWIN),
        "if entry.retire_only {": (1, _TWIN),
        "let decision = stop_ports::decide(|| match kind {": (1, _TWIN),
        "return start_ports::release_and_reply(": (2, _TWIN),
        "run_phase(": (4, _TWIN),
        "run_phase(&mut executor(&ports, tail), Placement::PostDT, None, rid);": (1, _TWIN),
        "let decision = capture_stop.consume(|op_status| {": (1, _TWIN),
    },
    f"{TAURI}/src/core_history.rs": {
        "pub(crate) fn consume<R>(&mut self, decide: impl FnOnce(OpStatus) -> R) -> R {": (1, _TWIN + "; the twin also reports the status it saw"),
    },
    f"{TAURI}/src/dictation_executor/executor.rs": {
        "FieldAdapter::cancel_through(ports.app_state(), id.0);": (1, _TWIN),
        "FieldAdapter::clear_context(ports.app_state(), id.0);": (1, _TWIN),
        "FieldAdapter::set_context(ports.app_state(), id.0, snapshot);": (1, _TWIN),
    },
}

_PRODUCTION = re.compile(rf"^{re.escape(TAURI)}/(src|crates/[^/]+/src)/.+\.rs$")


def is_production_path(path: str) -> bool:
    """A Rust source file of the app or of one of its crates."""
    return bool(_PRODUCTION.match(path))


def is_test_file(path: str, text: str) -> bool:
    name = path.rsplit("/", 1)[-1]
    return (
        name.endswith("_tests.rs")
        or name == "tests.rs"
        or "/tests/" in path
        or path in TEST_ONLY_FILES
        or re.search(r"^#!\[cfg\(test\)\]", text, re.M) is not None
    )


def is_shadow_only(path: str) -> bool:
    return any(path == entry or (entry.endswith("/") and path.startswith(entry)) for entry in SHADOW_ONLY)


def numstat(root: Path, base: str) -> dict[str, tuple[int, int]]:
    """`path -> (added, removed)` of the branch against its merge base with `base`.

    Renames are not followed: a renamed file is the deletion of its old path and the
    addition of its new one, so a rename cannot hide a removal.
    """
    result = subprocess.run(
        ["git", "diff", "--numstat", "--no-renames", f"{base}...HEAD"],
        cwd=root,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"git diff --numstat {base}...HEAD failed: {result.stderr.strip()}")
    stats: dict[str, tuple[int, int]] = {}
    for line in result.stdout.splitlines():
        added, removed, path = line.split("\t", 2)
        # A binary file reports `-`: it is never a source file.
        stats[path] = (
            int(added) if added.isdigit() else 0,
            int(removed) if removed.isdigit() else 0,
        )
    return stats


def show(root: Path, ref: str, path: str) -> str:
    """The file's text at `ref` (empty if it does not exist there)."""
    result = subprocess.run(
        ["git", "show", f"{ref}:{path}"], cwd=root, capture_output=True, text=True
    )
    return result.stdout if result.returncode == 0 else ""


def guarded_paths(stats: dict[str, tuple[int, int]], root: Path, base: str) -> list[str]:
    """The changed production files the shadow may only insert into."""
    guarded = []
    for path in sorted(stats):
        if not is_production_path(path) or is_shadow_only(path):
            continue
        # A file is a test file by what the base says it is: adding `#![cfg(test)]` to an
        # existing production file must not exempt it. A new file has no base text.
        text = show(root, base, path) or show(root, "HEAD", path)
        if is_test_file(path, text):
            continue
        guarded.append(path)
    return guarded


_SHADOW_FEATURE = re.compile(r'feature\s*=\s*"dictation-shadow"')


def _cfg_value(predicate: str, shadow: bool | None = None) -> bool | None:
    """The constant value of a `cfg` predicate: `True`, `False`, or `None` when it depends on
    the build (a feature, a target, `test`). With `shadow` given, the shadow feature itself is
    that constant: `_cfg_value(p, shadow=True)` is the predicate in a feature build."""
    predicate = predicate.strip()
    if shadow is not None and _SHADOW_FEATURE.fullmatch(predicate):
        return shadow
    match = re.fullmatch(r"(all|any|not)\((.*)\)", predicate, re.S)
    if not match:
        return None
    kind, inner = match.groups()
    parts, depth, current = [], 0, ""
    for char in inner:
        if char == "," and depth == 0:
            parts.append(current)
            current = ""
            continue
        depth += {"(": 1, ")": -1}.get(char, 0)
        current += char
    if current.strip():
        parts.append(current)
    values = [_cfg_value(part, shadow) for part in parts]
    if kind == "not":
        return None if len(values) != 1 or values[0] is None else not values[0]
    if kind == "all":
        if any(value is False for value in values):
            return False
        return True if all(value is True for value in values) else None
    if any(value is True for value in values):
        return True
    return False if all(value is False for value in values) else None


def _strip_comments(line: str) -> str:
    """The line without `//` and `/* */` comments (outside string literals)."""
    out: list[str] = []
    i, in_string = 0, False
    while i < len(line):
        char = line[i]
        if in_string:
            out.append(char)
            if char == "\\" and i + 1 < len(line):
                out.append(line[i + 1])
                i += 2
                continue
            if char == '"':
                in_string = False
        elif char == '"':
            in_string = True
            out.append(char)
        elif line.startswith("//", i):
            break
        elif line.startswith("/*", i):
            end = line.find("*/", i + 2)
            if end < 0:
                break
            i = end + 2
            continue
        else:
            out.append(char)
        i += 1
    return "".join(out)


def _requires_test(predicate: str) -> bool:
    """The predicate holds only in a test build (`test`, or `all(.., test, ..)`)."""
    predicate = predicate.strip()
    if predicate == "test":
        return True
    match = re.fullmatch(r"(all|any)\((.*)\)", predicate, re.S)
    if not match:
        return False
    kind, inner = match.groups()
    parts, depth, current = [], 0, ""
    for char in inner:
        if char == "," and depth == 0:
            parts.append(current)
            current = ""
            continue
        depth += {"(": 1, ")": -1}.get(char, 0)
        current += char
    if current.strip():
        parts.append(current)
    flags = [_requires_test(part) for part in parts]
    return bool(flags) and (any(flags) if kind == "all" else all(flags))


def cfg_attributes(text: str) -> list[tuple[int, int, bool, str]]:
    """Every `#[cfg(..)]` / `#![cfg(..)]` of `text`, comments stripped first, as
    `(first line, line after the attribute's end, is_inner, predicate)` (1-based). An
    attribute may span lines and share a line with the item it gates."""
    lines = [_strip_comments(line) for line in text.splitlines()]
    code = "\n".join(lines)
    found = []
    for match in re.finditer(r"#(!?)\[\s*cfg\s*\(", code):
        depth, at = 1, match.end()
        while at < len(code) and depth:
            depth += {"(": 1, ")": -1}.get(code[at], 0)
            at += 1
        predicate = code[match.end() : at - 1]
        rest = code[at:].lstrip()
        if depth or not rest.startswith("]"):
            continue
        end = at + (len(code[at:]) - len(rest)) + 1
        found.append(
            (
                code.count("\n", 0, match.start()) + 1,
                code.count("\n", 0, end) + 1,
                match.group(1) == "!",
                predicate,
            )
        )
    return found


def added_lines(root: Path, base: str, path: str) -> set[int]:
    """The 1-based line numbers the branch added to `path`."""
    result = subprocess.run(
        ["git", "diff", "-U0", "--no-renames", f"{base}...HEAD", "--", path],
        cwd=root,
        capture_output=True,
        text=True,
    )
    lines: set[int] = set()
    for header in re.finditer(r"^@@ -\S+ \+(\d+)(?:,(\d+))? @@", result.stdout, re.M):
        start, count = int(header.group(1)), int(header.group(2) or 1)
        lines.update(range(start, start + count))
    return lines


def disabled_originals(
    root: Path,
    base: str,
    path: str,
    allow: dict[str, dict[str, tuple[int, str]]] | None = None,
) -> list[str]:
    """Gates the branch put on code the base already had, that switch it off in a production
    build: an always-false gate, a gate that holds only in a test build, and any file-level
    (`#![cfg(..)]`) gate added to a file the base had; and a gate that is false in the
    feature build (`cfg(not(feature = "dictation-shadow"))`), unless the statement is named
    in `SHADOW_GATED_OFF_ALLOW`."""
    allow = SHADOW_GATED_OFF_ALLOW if allow is None else allow
    permitted = {line: count for line, (count, _) in allow.get(path, {}).items()}
    head = show(root, "HEAD", path)
    text = head.splitlines()
    added = added_lines(root, base, path)
    in_base = bool(show(root, base, path))
    found = []
    for start, after, inner, predicate in cfg_attributes(head):
        if start not in added:
            continue
        shown = f"#{'!' if inner else ''}[cfg({predicate.strip()})]"
        if inner:
            if in_base:
                found.append(
                    f"{path}:{start}: `{shown}` gates the whole of a file the base already had"
                )
            continue
        always_off = _cfg_value(predicate) is False or _requires_test(predicate)
        off_in_feature = _cfg_value(predicate, shadow=True) is False
        if not always_off and not off_in_feature:
            continue
        # The item the attribute gates: the next line that is not another attribute or a comment.
        target = after + 1
        while target <= len(text) and re.match(r"^\s*(#\[|#!\[|//|$)", text[target - 1]):
            target += 1
        # An item on the attribute's own line is new text with it.
        if target <= len(text) and target not in added:
            statement = text[target - 1].strip()
            if not always_off and permitted.get(statement, 0) > 0:
                permitted[statement] -= 1
                continue
            where = "" if always_off else " in the feature build"
            found.append(
                f"{path}:{start}: `{shown}` disables the existing statement "
                f"at line {target}{where} without deleting it"
            )
    return found


def violations(
    stats: dict[str, tuple[int, int]],
    guarded: list[str] | None = None,
    allow: dict[str, tuple[int, str]] | None = None,
) -> list[str]:
    allow = SHADOW_DIFF_ALLOW if allow is None else allow
    if guarded is None:
        guarded = [
            path for path in stats if is_production_path(path) and not is_shadow_only(path)
        ]
    found = []
    for path in guarded:
        added, removed = stats.get(path, (0, 0))
        permitted = allow.get(path, (0, ""))[0]
        if removed > permitted:
            found.append(
                f"{path}: removes {removed} lines (+{added}/-{removed}); the shadow may only "
                f"insert in a production file ({permitted} allowed)"
            )
    for path, (limit, reason) in allow.items():
        if not reason.strip():
            found.append(f"SHADOW_DIFF_ALLOW[{path}] gives no reason")
        if not is_production_path(path):
            found.append(f"SHADOW_DIFF_ALLOW[{path}] names a file that is not a production file")
    return found


def main(root: Path = ROOT, base: str = "origin/main") -> int:
    try:
        stats = numstat(root, base)
    except RuntimeError as error:
        print(error, file=sys.stderr)
        return 2
    guarded = guarded_paths(stats, root, base)
    problems = violations(stats, guarded)
    for path in guarded:
        problems.extend(disabled_originals(root, base, path))
    for path, statements in SHADOW_GATED_OFF_ALLOW.items():
        for statement, (count, reason) in statements.items():
            if count < 1 or not reason.strip():
                problems.append(f"SHADOW_GATED_OFF_ALLOW[{path}][{statement!r}] needs a count and a reason")
        if not is_production_path(path):
            problems.append(f"SHADOW_GATED_OFF_ALLOW[{path}] names a file that is not a production file")
    for path in guarded:
        added, removed = stats[path]
        print(f"{added}\t{removed}\t{path}")
    if problems:
        for problem in problems:
            print(f"error: {problem}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", default="origin/main")
    arguments = parser.parse_args()
    sys.exit(main(ROOT, arguments.base))
