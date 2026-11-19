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

Inserted attributes on an existing statement or item are a closed list (D-S2c-8 amendment
1, D-S2c-12 c). Nothing is evaluated: the one `cfg` accepted is exactly
`#[cfg(not(feature = "dictation-shadow"))]` (whitespace and comments aside) on a statement
named in `SHADOW_TWINS` (path, the statement's first line, a form, and why); every other
inserted `cfg` or `cfg_attr` there is rejected, however it is spelled (`not (`, a raw string,
`any(not(..), ..)`, `cfg_attr(.., cfg(..))`), and so is any other attribute except the
named `SHADOW_ATTR_ALLOW` entries. A named statement's feature twin (the one statement under
`#[cfg(feature = "dictation-shadow")]` that follows it) must be its original plus at most one
permitted insertion (`closed`), a pinned reviewed pair (`frozen`, debt against D-S2c-12 b), or
a `generate_handler!` entry's wrapper (`handler`).

A file's additions are checked by the Rust tests (`shadow_off_by_default` reads every
`cfg` attribute): this check is the other half, that nothing was taken out.
"""

from __future__ import annotations

import argparse
import ast
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
SHADOW_DIFF_ALLOW: dict[str, tuple] = {
    f"{TAURI}/src/main_thread_watch.rs": (
        1,
        "test-only: the `tests` module parses `lib.rs`'s `generate_handler!` list up to the "
        "first `]`; the shadow's `#[cfg(..)]` entries inside the macro carry their own `]`, so "
        "that one line now stops at the macro's closing `]))`. No production statement changes.",
        # The one line it may remove, exactly (a third element is optional: without it the file
        # may remove `limit` lines of any text). Every other line of the file is checked like any
        # other production file's: scoping and the base tokens surviving.
        ['let end = start + lib_rs[start..].find(\']\').expect("handler end");'],
    ),
}

# Existing statements the feature build may switch off, because a feature twin runs in their
# place: `path -> [entry]`, an entry being `{"first": the statement's first line (stripped),
# "form": "handler" | "closed" | "frozen", "why": ..., "pair": digest (frozen only)}`. A gate
# on any other existing statement is rejected, and so is a second gate on one entry's.
#
# * `handler`: a `generate_handler!` entry of `lib.rs`; the wrapper command of the same name
#   is inserted with the opposite gate and calls the original (D-S2c-8 amendment 1).
# * `closed`: the twin is checked mechanically: the original statement plus nothing, or plus
#   exactly one insertion of the closed list (D-S2c-12 b): a trailing
#   `.attach_flight(<ident>)` or one extra trailing `<ident>` argument. (A single
#   `shadow::..(<idents>);` statement beside a twin is a plain gated insertion, which needs no
#   twin at all.) Argument order, `?`, block boundaries, `let` order and a clock read's place
#   must be the original's, token for token.
# * `frozen`: the twin is an observation of a value the original discards or of a lock's
#   scope, which the closed list cannot express. The pair (both token streams) is pinned by
#   digest: any change to either side fails until a reviewer pins the new digest. Each is
#   debt against D-S2c-12 (b), listed by name so none is added unseen.
# * `frozen_fn`: the shadow's copy of a whole live function (`*_observed`), pinned against its
#   original by the digest of both token streams (D-S2c-15 C3). Here `first` is the copy's first
#   line and `original` the original function's. Editing an original without re-pinning fails.
# A `handler` entry may carry `retire_imports`: the exact `use` statements, written out by a
# reviewer, that only its wrapper in `shadow_commands.rs` needed. When a cutover retires the
# entry, the wrapper's lines and exactly those statements may leave the feature-only module
# without making the PR a shadow change. Nothing is inferred (a trait import changes a method
# call's binding without being named), and any other deleted `use` is a shadow edit. The list
# is read from the **base** table, so it is added by an earlier reviewed PR, never by the
# cutover itself. An item is compared whole (visibility and attributes included), and a line
# is exempt only when every token on it belongs to a listed item or a retired wrapper.
# Today: `SmartAutoRequest` is the start wrapper's and `audio_lifecycle` the stop wrapper's
# (each has its own `use` statement); `State` and `commanded` are shared, so unlisted.
SHADOW_TWINS: dict[str, list[dict]] = {
    "app/src-tauri/src/commands/recording.rs": [
        {"first": "fn spawn_dictation_partial_ticker(", "form": "frozen", "pair": "3ec58dd4b5441fa6", "why": "the shared ticker lookup (`ticker_start_check`) replaces the spawner's head, so the skip taken is the witness of that one lookup (D-S2c-10 R2)"},
        {"first": "self.history.ret_pipeline(self.rid);", "form": "frozen", "pair": "b9e8f6659edbbaf5", "why": "keeps the answer of the H2 retirement, which the original discards, for the flight's H2 witness"},
        {"first": "let pre = stop_ports::try_run_phase(", "form": "closed", "why": 'the original plus `.attach_flight(<flight>)` on its executor, the one insertion of the closed list (D-S2c-12 b)'},
        {"first": "stop_ports::run_phase(", "form": "closed", "why": 'the original plus `.attach_flight(<flight>)` on its executor, the one insertion of the closed list (D-S2c-12 b)'},
        {"first": "if entry.retire_only {", "form": "frozen", "pair": "4f976775a87ff93f", "why": 'the finish a drain runs has no reply of its own: it runs inside a no-reply reply scope (D-S2c-10 R6)'},
        {"first": "stop_ports::run_phase(", "form": "closed", "why": 'the original plus `.attach_flight(<flight>)` on its executor, the one insertion of the closed list (D-S2c-12 b)'},
        {"first": "return start_ports::release_and_reply(", "form": "frozen", "pair": "7da04592c2be829b", "why": "armed: the observed helper attaches the active flight to the refusal's PT:REL executor, with the executor gone before the reply is rendered; disarmed: the original helper (D-S2c-12 a)"},
        {"first": "return start_ports::release_and_reply(", "form": "frozen", "pair": "fbe267b795db0df3", "why": "armed: the observed helper attaches the active flight to the refusal's PT:REL executor, with the executor gone before the reply is rendered; disarmed: the original helper (D-S2c-12 a)"},
        {"first": "run_phase(", "form": "closed", "why": 'the original plus `.attach_flight(<flight>)` on its executor, the one insertion of the closed list (D-S2c-12 b)'},
        {"first": "run_phase(", "form": "closed", "why": 'the original plus `.attach_flight(<flight>)` on its executor, the one insertion of the closed list (D-S2c-12 b)'},
        {"first": "run_phase(", "form": "closed", "why": 'the original plus `.attach_flight(<flight>)` on its executor, the one insertion of the closed list (D-S2c-12 b)'},
        {"first": "run_phase(&mut executor(&ports, tail), Placement::PostDT, None, rid);", "form": "closed", "why": 'the original plus `.attach_flight(<flight>)` on its executor, the one insertion of the closed list (D-S2c-12 b)'},
        {"first": "run_phase(", "form": "closed", "why": 'the original plus `.attach_flight(<flight>)` on its executor, the one insertion of the closed list (D-S2c-12 b)'},
         {"first": 'pub(crate) fn register_partial_ticker_locked_observed(', "form": "frozen_fn", "original": 'fn register_partial_ticker_locked(', "pair": 'b2176680c1f4af95', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3)"},
    ],
    "app/src-tauri/src/commands/dictation_preview.rs": [
        {"first": 'pub(crate) fn note_ticker_started_observed(recording_id: u64) -> (bool, u64) {', "form": "frozen_fn", "original": 'pub(crate) fn note_ticker_started(recording_id: u64) -> bool {', "pair": '19181aeafcb0c143', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3)"},
        {"first": 'pub(crate) fn ticker_started_observed(&self, recording_id: u64) -> (bool, u64) {', "form": "frozen_fn", "original": 'pub(crate) fn ticker_started(&self, recording_id: u64) -> bool {', "pair": '891cc929ccfeb863', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3)"},
    ],
    "app/src-tauri/src/state.rs": [
        {"first": 'pub(crate) fn set_active_context_observed(', "form": "frozen_fn", "original": 'pub fn set_active_context(', "pair": 'bba485a25404b92e', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3)"},
        {"first": 'pub(crate) fn clear_active_context_observed(', "form": "frozen_fn", "original": 'pub fn clear_active_context(&self, recording_id: u64) -> bool {', "pair": 'e80856b7cea0ca48', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3)"},
        {"first": 'pub(crate) fn cancel_recording_observed(', "form": "frozen_fn", "original": 'pub fn cancel_recording(&self, id: u64) {', "pair": '117a6443b03f22eb', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3)"},
    ],
    "app/src-tauri/src/core_history.rs": [
        {"first": "pub(crate) fn consume<R>(&mut self, decide: impl FnOnce(OpStatus) -> R) -> R {", "form": "frozen", "pair": "dcc7056f2f3ceb6a", "why": "keeps the answer of the H1 retirement, which the original discards, for the flight's H1 witness"},
         {"first": 'pub(crate) fn consume_observed<R>(', "form": "frozen_fn", "original": 'pub(crate) fn consume<R>(&self, token: Continuation, decide: impl FnOnce(OpStatus) -> R) -> R {', "pair": '9f7ab9a1ba3118da', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3)"},
    ],
    "app/src-tauri/src/dictation_executor/executor.rs": [
        {"first": "FieldAdapter::cancel_through(ports.app_state(), id.0);", "form": "frozen", "pair": "c4e1922cc096db96", "why": 'an observing flight takes the mirror witness the cancel returns (the fetch_max result the original discards)'},
        {"first": "FieldAdapter::clear_context(ports.app_state(), id.0);", "form": "frozen", "pair": "c865ad563d1e19a2", "why": 'an observing flight takes the mirror witness, built after the leaf guard is released (D-S2c-12 a); otherwise the original'},
        {"first": "FieldAdapter::set_context(ports.app_state(), id.0, snapshot);", "form": "frozen", "pair": "1fa8e76a377b9034", "why": 'an observing flight takes the mirror witness, built after the leaf guard is released (D-S2c-12 a); otherwise the original'},
        {"first": 'fn detached_observed(', "form": "frozen_fn", "original": 'fn detached(&self, effect: &Effect) -> PortResult;', "pair": '8c0dec6ac06f57d7', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3, D-S2c-15 round 2)"},
        {"first": 'fn detached_under_d_observed(', "form": "frozen_fn", "original": 'fn detached_under_d(&self, effect: &Effect, _dictation: &DictationState) -> PortResult {', "pair": '57ac298d6fc44deb', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3, D-S2c-15 round 2)"},
    ],
    "app/src-tauri/src/commands/recording/start_ports.rs": [
        {"first": 'pub(super) fn release_and_reply_observed(', "form": "frozen_fn", "original": 'pub(super) fn release_and_reply(', "pair": '4ceece00dd7bb5fb', "why": "the armed refusal helper (the PT:REL run with the flight attached) beside the original helper every disarmed refusal takes: editing the original (its reply, the smart-auto error) without the copy, or the copy alone, fails until both are re-pinned (C3)"},
    ],
    "app/src-tauri/src/commands/recording/lifecycle_ports.rs": [
        {"first": 'fn detached_observed(', "form": "frozen_fn", "original": 'fn detached(&self, effect: &Effect) -> PortResult {', "pair": 'e52189b0f63f9c9b', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3, D-S2c-15 round 2)"},
    ],
    "app/src-tauri/src/commands/recording/ready_ports.rs": [
        {"first": 'fn detached_under_d_observed(', "form": "frozen_fn", "original": 'fn detached_under_d(&self, effect: &Effect, dictation: &DictationState) -> PortResult {', "pair": 'adb2f953a951b906', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3, D-S2c-15 round 2)"},
    ],
    "app/src-tauri/src/dictation_executor/adapter.rs": [
        {"first": 'pub(crate) fn cancel_through_observed(', "form": "frozen_fn", "original": 'pub(crate) fn cancel_through(app: &AppState, id: u64) {', "pair": '9054b40228d09f7e', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3, D-S2c-15 round 2)"},
        {"first": 'pub(crate) fn set_context_observed(', "form": "frozen_fn", "original": 'pub(crate) fn set_context(', "pair": '93c80bc579b75441', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3, D-S2c-15 round 2)"},
        {"first": 'pub(crate) fn clear_context_observed(', "form": "frozen_fn", "original": 'pub(crate) fn clear_context(app: &AppState, id: u64) -> bool {', "pair": '1eeb6a4ee4aae1ed', "why": "the shadow's observed copy of a live function: it must stay the original plus the shadow's own observation, so editing either side needs a reviewed new digest (C3, D-S2c-15 round 2)"},
    ],
    "app/src-tauri/src/lib.rs": [
        {"first": "commands::recording::process_audio,", "form": "handler", "twin": "commands::recording::shadow_commands::process_audio_witnessed,", "why": 'the wrapper command of the same name is inserted with the opposite gate and calls this one (D-S2c-8 amendment 1)'},
        {"first": "commands::recording::start_native_recording,", "form": "handler", "retire_imports": ["use super::SmartAutoRequest;"], "twin": "commands::recording::shadow_commands::start_native_recording_witnessed,", "why": 'the wrapper command of the same name is inserted with the opposite gate and calls this one (D-S2c-8 amendment 1)'},
        {"first": "commands::recording::stop_native_recording,", "form": "handler", "retire_imports": ["use super::audio_lifecycle;"], "twin": "commands::recording::shadow_commands::stop_native_recording_witnessed,", "why": 'the wrapper command of the same name is inserted with the opposite gate and calls this one (D-S2c-8 amendment 1)'},
        {"first": "commands::recording::cancel_native_recording,", "form": "handler", "twin": "commands::recording::shadow_commands::cancel_native_recording_witnessed,", "why": 'the wrapper command of the same name is inserted with the opposite gate and calls this one (D-S2c-8 amendment 1)'},
    ],
}

# The one attribute other than the twin gate that an inserted line may put on an existing item:
# `path -> [{"attr": the attribute (whitespace removed), "first": the item's first line, "why"}]`.
# It is not a `cfg`: it cannot disable or alter a statement. Any other inserted attribute on an
# existing statement or item is rejected (D-S2c-12 c), a `cfg` or `cfg_attr` always, and
# without being evaluated.
SHADOW_ATTR_ALLOW: dict[str, list[dict]] = {
    f"{TAURI}/src/lib.rs": [
        {
            "attr": "#[track_caller]",
            "first": "fn lock_or_recover(&self) -> MutexGuard<'_, T>;",
            "why": "the dictation-lock guard function reports its caller's location to the "
            "shadow's lock counter; `track_caller` changes no behaviour",
        },
        {
            "attr": "#[track_caller]",
            "first": "fn lock_or_recover(&self) -> MutexGuard<'_, T> {",
            "why": "the implementation of the trait method above",
        },
    ],
}

# Inserted code a shadow change may add with no gate, because it is a pure function nothing in
# the live path calls: `path -> [{"first": the item's first line (stripped), "why"}]`. Each is a
# reviewed exception to "every inserted statement is behind the feature" (C2 i), named one by
# one. The core crate has no shadow feature to gate on.
SHADOW_UNGATED_ALLOW: dict[str, list[dict]] = {
    f"{TAURI}/crates/dictation-core/src/decision.rs": [
        {
            "first": "pub fn ordinal_inline(self) -> u16 {",
            "why": "a pure twin of `RowId::ordinal` that allocates nothing, called only by the "
            "shadow's capture; the core crate has no shadow feature (Stage 2c-2)",
        },
    ],
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


# ----------------------------------------------------------------------------- tokens


_TOKEN = re.compile(
    r"""
    (?P<comment>//[^\n]*|/\*.*?\*/)
  | (?P<raw>b?r(?P<hashes>\#*)"(?:.|\n)*?"(?P=hashes))
  | (?P<string>b?"(?:\\.|[^"\\])*")
  | (?P<char>b?'(?:\\.|[^\\'])')
  | (?P<lifetime>'[A-Za-z_]\w*)
  | (?P<ident>[A-Za-z_]\w*)
  | (?P<number>\d[\w.]*)
  | (?P<punct>::|->|=>|==|!=|<=|>=|&&|\|\||\.\.=?|<<=?|>>=?|[-+*/%&|^]=|\S)
    """,
    re.X | re.S,
)


def tokenize(text: str) -> list[tuple[str, int]]:
    """`(token, 1-based line)` of the Rust text, comments and whitespace dropped. A string,
    a raw string (`r#"..."#`) and a character are one token each, so a gate cannot hide in
    one and spelling a string differently changes its token."""
    tokens: list[tuple[str, int]] = []
    line = 1
    at = 0
    while at < len(text):
        match = _TOKEN.match(text, at)
        if match is None:
            if text[at] == "\n":
                line += 1
            at += 1
            continue
        if match.lastgroup != "comment":
            tokens.append((match.group(), line))
        line += match.group().count("\n")
        at = match.end()
    return tokens


_OPEN, _CLOSE = "([{", ")]}"
_BLOCK_LIKE = {"if", "match", "for", "while", "loop", "unsafe", "{"}
_ITEM = {"fn", "pub", "async", "impl", "mod", "struct", "enum", "trait", "extern"}


def attribute_at(tokens: list[tuple[str, int]], at: int) -> int | None:
    """The index after the attribute starting at `at` (`#[..]` or `#![..]`), or `None`."""
    if at >= len(tokens) or tokens[at][0] != "#":
        return None
    nxt = at + 1
    if nxt < len(tokens) and tokens[nxt][0] == "!":
        nxt += 1
    if nxt >= len(tokens) or tokens[nxt][0] != "[":
        return None
    depth = 0
    for k in range(nxt, len(tokens)):
        depth += {"[": 1, "]": -1}.get(tokens[k][0], 0)
        if depth == 0:
            return k + 1
    return None


def statement_end(tokens: list[tuple[str, int]], start: int) -> int:
    """The index after the statement or item beginning at `start`: its `;` at depth 0, or the
    `}` that closes a block-like statement or an item (not followed by `else`)."""
    first = tokens[start][0]
    block_like = first in _BLOCK_LIKE or first in _ITEM
    depth = 0
    for k in range(start, len(tokens)):
        token = tokens[k][0]
        if token in _OPEN:
            depth += 1
        elif token in _CLOSE:
            depth -= 1
            if depth < 0:
                return k
            if depth == 0 and token == "}" and block_like:
                if k + 1 < len(tokens) and tokens[k + 1][0] == "else":
                    continue
                return k + 1
        elif token == ";" and depth == 0:
            return k + 1
    return len(tokens)


def normalised(tokens: list[str]) -> list[str]:
    """The tokens without a trailing comma (`,` right before a closing bracket): rustfmt adds
    or removes it when it reflows a call, and it changes nothing."""
    return [
        token
        for k, token in enumerate(tokens)
        if not (token == "," and k + 1 < len(tokens) and tokens[k + 1] in _CLOSE)
    ]


def pair_hash(original: list[str], twin: list[str]) -> str:
    """A stable digest of an original statement and its feature twin: a frozen entry names
    the pair a reviewer accepted, so changing either side needs a new, reviewed digest."""
    import hashlib

    body = "\x1f".join(original) + "\x1e" + "\x1f".join(twin)
    return hashlib.sha256(body.encode()).hexdigest()[:16]


# --------------------------------------------------------------- twin equivalence (b)

_IDENT = r"[A-Za-z_]\w*"
_ARG = rf"&? ?{_IDENT}(?: :: {_IDENT})*"
# The closed list of insertions that turn an original statement into its feature twin
# (D-S2c-12 b). Each is matched against the inserted run of tokens, joined by spaces.
_INSERTIONS = {
    "attach": re.compile(rf"\. attach_flight \( {_ARG} \)"),
    "argument": re.compile(rf", {_ARG}"),
}


def twin_difference(original: list[str], twin: list[str]) -> str | None:
    """`None` when `twin` is `original` plus nothing, or plus exactly one insertion of the
    closed list; otherwise what differs. Token-equal means argument order, `?`, block
    boundaries, `let` order and a clock read's place are all the original's."""
    if twin == original:
        return None
    extra = len(twin) - len(original)
    if extra <= 0:
        return "the twin is not the original plus one insertion (it is shorter or reordered)"
    # The same twin can be read as an insertion at several places when the inserted run
    # begins or ends with a token the original repeats: any reading that is a permitted
    # insertion is the reading.
    runs = []
    for at in range(len(original) + 1):
        if twin[:at] != original[:at] or twin[at + extra :] != original[at:]:
            continue
        run = " ".join(twin[at : at + extra])
        runs.append(run)
        after = twin[at + extra] if at + extra < len(twin) else ""
        for name, form in _INSERTIONS.items():
            if not form.fullmatch(run):
                continue
            if name == "attach" and (at == 0 or twin[at - 1] != ")"):
                continue
            if name == "argument" and after != ")":
                continue
            return None
    if runs:
        return f"the inserted tokens `{runs[0]}` are not one of the permitted insertions"
    return "the twin differs from the original by more than one insertion"


# --------------------------------------------------------------------- the allow list

# The only attribute an inserted gate on an existing statement or item may be: exactly this,
# after whitespace is normalised, on a statement named in `SHADOW_TWINS`. Any other inserted
# `cfg` or `cfg_attr` there is rejected without being evaluated (D-S2c-12 c).
NEGATED_GATE = '#[cfg(not(feature="dictation-shadow"))]'
FEATURE_GATE = '#[cfg(feature="dictation-shadow")]'


def added_lines_of(root: Path, base: str, path: str) -> set[int]:
    return added_lines(root, base, path)


def region_end(tokens: list[tuple[str, int]], start: int) -> int:
    """The index after the statement, item, list element or match arm at `start`."""
    end = statement_end(tokens, start)
    if end == 0 or end > len(tokens) or tokens[end - 1][0] == ";" or tokens[start][0] in _ITEM:
        return end
    depth = 0
    arrow = False
    for k in range(start, min(end, len(tokens))):
        token = tokens[k][0]
        if token in _OPEN:
            depth += 1
        elif token in _CLOSE:
            depth -= 1
        elif depth == 0 and token == "=>":
            arrow = True
        elif depth == 0 and token == ",":
            return k + 1
        if arrow and depth == 0 and token == "}":
            more = k + 1 < len(tokens) and tokens[k + 1][0] == ","
            return k + 2 if more else k + 1
    return end


def twin_regions(tokens: list[tuple[str, int]]) -> dict[int, dict]:
    """Every `#[cfg(not(feature = "dictation-shadow"))]` of the text, by the index of the
    statement it gates: the original statement, the feature twin that follows it (the one
    statement under `#[cfg(feature = "dictation-shadow")]`, if any) and their digest."""
    found: dict[int, dict] = {}
    at = 0
    while at < len(tokens):
        end = attribute_at(tokens, at)
        if end is None:
            at += 1
            continue
        if "".join(t for t, _ in tokens[at:end]) == NEGATED_GATE:
            start = end
            while (nxt := attribute_at(tokens, start)) is not None:
                start = nxt
            stop = region_end(tokens, start)
            # The attribute stack between the gate and the statement is part of the original
            # (and of the twin, below): a twin with another attribute stacked on it differs.
            original = normalised([t for t, _ in tokens[end:stop]])
            twin: list[str] | None = None
            twin_attr: int | None = None
            twin_stack = 0
            cursor = stop
            twin_end = attribute_at(tokens, cursor)
            if twin_end is not None and "".join(t for t, _ in tokens[cursor:twin_end]) == FEATURE_GATE:
                twin_attr = cursor
                begin = twin_end
                while (nxt := attribute_at(tokens, begin)) is not None:
                    begin = nxt
                twin = normalised([t for t, _ in tokens[twin_end : region_end(tokens, begin)]])
                twin_stack = begin - twin_end
            found[start] = {
                "gate": at,
                "stop": stop,
                "twin_stop": region_end(tokens, begin) if twin_attr is not None else None,
                "original": original,
                "twin": twin,
                "twin_attr": twin_attr,
                "twin_stack": twin_stack,
                "original_stack": start - end,
            }
        at = end
    return found


def base_survives(
    root: Path,
    base: str,
    path: str,
    tokens: list[tuple[str, int]],
    added: set[int],
    exempt: set[int] | None = None,
) -> list[str]:
    """D-S2c-10 amendment 3 / round 9 nit 4: take every inserted attribute and the statement a
    gated one governs out of the branch's tokens (a negated twin gate loses only its
    attribute, its statement stays): what is left must still hold the base's tokens, in
    order. A gated block that wraps an original statement removes it from the default
    build, and the removal shows here as base tokens that are gone."""
    base_tokens = tokenize(show(root, base, path))
    if exempt:
        # The named removed line of a `SHADOW_DIFF_ALLOW` entry is meant to go.
        base_tokens = [(text, line) for text, line in base_tokens if line not in exempt]
    if not base_tokens:
        return []
    drop: set[int] = set()
    at = 0
    while at < len(tokens):
        first_end = attribute_at(tokens, at)
        if first_end is None:
            at += 1
            continue
        stack = [(at, first_end)]
        follow = first_end
        while (nxt := attribute_at(tokens, follow)) is not None:
            stack.append((follow, nxt))
            follow = nxt
        inserted = [(a, b) for a, b in stack if tokens[a][1] in added]
        if inserted:
            compact = ["".join(t for t, _ in tokens[a:b]) for a, b in inserted]
            for a, b in inserted:
                drop.update(range(a, b))
            gates = any(re.match(r"#!?\[(cfg|cfg_attr)\b", c) for c in compact)
            inner = any(c.startswith("#!") for c in compact)
            if gates and not inner and NEGATED_GATE not in compact and follow < len(tokens):
                drop.update(range(follow, region_end(tokens, follow)))
        at = follow
    kept = [text for k, (text, _) in enumerate(tokens) if k not in drop]
    cursor = 0
    for text, line in base_tokens:
        while cursor < len(kept) and kept[cursor] != text:
            cursor += 1
        if cursor == len(kept):
            return [
                f"{path}:{line}: the base's `{text}` is gone from the branch once its gated "
                "insertions are taken out: a gated region encloses text the base already had"
            ]
        cursor += 1
    return []


def removed_lines(root: Path, base: str, path: str) -> set[int]:
    """The 1-based line numbers (in the base) the branch removed from `path`."""
    result = subprocess.run(
        ["git", "diff", "-U0", "--no-renames", f"{base}...HEAD", "--", path],
        cwd=root,
        capture_output=True,
        text=True,
    )
    lines: set[int] = set()
    for header in re.finditer(r"^@@ -(\d+)(?:,(\d+))? \+\S+ @@", result.stdout, re.M):
        start, count = int(header.group(1)), int(header.group(2) or 1)
        lines.update(range(start, start + count))
    return lines


# ------------------------------------------------------------------ what a token is under

_SHADOW_WORD = "dictation-shadow"
# Identifiers that name the shadow even when no `cfg` gates the line that uses them.
_SHADOW_IDENTS = {"shadow_site", "dictation_shadow"}


def _mentions_shadow(compact_tokens: list[str]) -> bool:
    return any(_SHADOW_WORD in token for token in compact_tokens)


def _predicate_of(tokens: list[tuple[str, int]], at: int, end: int) -> str | None:
    """The text inside `cfg(..)` of the attribute `tokens[at:end]`, or `None` for any other
    attribute (`cfg_attr` included: it never gates a statement by itself)."""
    compact = "".join(t for t, _ in tokens[at:end])
    match = re.fullmatch(r"#\[cfg\((.*)\)\]", compact, re.S)
    return match.group(1) if match else None


def _negates_shadow(tokens: list[tuple[str, int]], at: int, end: int) -> bool:
    """The attribute names the shadow feature under an odd number of `not(..)`: it switches
    its item off in a feature build, however the feature is spelled (a raw string included)."""
    frames: list[bool] = []  # parity of `not` for each open parenthesis
    parity = False
    pending_not = False
    for k in range(at, end):
        token = tokens[k][0]
        if token == "not":
            pending_not = True
        elif token == "(":
            parity = parity != pending_not
            frames.append(pending_not)
            pending_not = False
        elif token == ")":
            if frames:
                parity = parity != frames.pop()
        elif _SHADOW_WORD in token and parity:
            return True
        else:
            pending_not = False
    return False


def classify(
    tokens: list[tuple[str, int]],
    lines: list[str] | None = None,
    path: str | None = None,
    table: dict[str, list[dict]] | None = None,
) -> tuple[list[str | None], set[int]]:
    """What each token is under, and which tokens belong to an attribute.

    The mark of a token is one of

    * `feature`: it is in the statement or item a `#[cfg(..)]` governs that is off in a
      default build because the shadow feature is off (`feature = "dictation-shadow"`, or an
      `all(..)` with it), or it is that gate itself;
    * `mixed`: a gate that names the shadow but is not off in a default build by that alone
      (`any(feature = .., unix)`, a raw-string spelling): shadow text, not behind the feature;
    * `test`: it is in a statement that exists only in a test build;
    * `twin`: it is a negated twin gate or the feature twin it governs (D-S2c-12 b);
    * `orig`: it is the original statement a negated twin gate governs;
    * `macro`: a `shadow_site!(..)` call (empty without the feature) or an
      `if dictation_shadow::enabled() { .. }` block with no `else`, which runs only when the
      shadow is armed;
    * `None`: ordinary code.
    """
    n = len(tokens)
    marks: list[str | None] = [None] * n
    attributes: set[int] = set()

    def put(a: int, b: int, kind: str) -> None:
        for k in range(a, min(b, n)):
            if marks[k] is None:
                marks[k] = kind

    # A pinned function and its observed copy are a twin pair too (C3): editing either is the
    # pin's business, not a shadow change's.
    # `table` is the twin table the text was written under: the base's, for the base's text (a
    # complete retirement removes its entries, and the base's copies are still twins then).
    attribute_starts: dict[int, int] = {}
    cursor = 0
    while cursor < n:
        attr_end = attribute_at(tokens, cursor)
        if attr_end is None:
            cursor += 1
            continue
        attribute_starts[attr_end] = cursor
        cursor = attr_end
    for entry in (SHADOW_TWINS if table is None else table).get(path or "", []):
        if entry["form"] != "frozen_fn" or lines is None:
            continue
        for name, kind in ((entry["original"], "orig"), (entry["first"], "twin")):
            for k, (_, line) in enumerate(tokens):
                if (k == 0 or tokens[k - 1][1] != line) and first_line_of(lines, line) == name:
                    # The attributes in front of the function (its feature gate) go with it.
                    first = k
                    while first in attribute_starts:
                        first = attribute_starts[first]
                    put(first, statement_end(tokens, k), kind)
                    break
    for region in twin_regions(tokens).values():
        gate_end = attribute_at(tokens, region["gate"]) or region["gate"]
        put(region["gate"], gate_end, "twin")
        put(gate_end, region["stop"], "orig")
        if region["twin_attr"] is not None:
            put(region["twin_attr"], region["twin_stop"], "twin")
    at = 0
    while at < n:
        first_end = attribute_at(tokens, at)
        if first_end is None:
            at += 1
            continue
        stack = [(at, first_end)]
        follow = first_end
        while (nxt := attribute_at(tokens, follow)) is not None:
            stack.append((follow, nxt))
            follow = nxt
        for a, b in stack:
            attributes.update(range(a, b))
        kind: str | None = None
        for a, b in stack:
            predicate = _predicate_of(tokens, a, b)
            if predicate is None:
                continue
            if _mentions_shadow([t for t, _ in tokens[a:b]]):
                if _cfg_value(predicate, shadow=False) is False:
                    kind = "feature"
                elif kind is None:
                    kind = "mixed"
            elif kind is None and _requires_test(predicate):
                kind = "test"
        inner = tokens[at][0] == "#" and at + 1 < n and tokens[at + 1][0] == "!"
        if kind is not None and not inner and follow < n:
            put(at, region_end(tokens, follow), kind)
        at = follow
    for k in range(n):
        text = tokens[k][0]
        previous = tokens[k - 1][0] if k else ";"
        if text == "shadow_site" and k + 1 < n and tokens[k + 1][0] == "!":
            start = k - 2 if k >= 2 and tokens[k - 1][0] == "::" and tokens[k - 2][0] == "crate" else k
            put(start, statement_end(tokens, start), "macro")
        elif text == "if" and previous in (";", "{", "}"):
            head = [t for t, _ in tokens[k + 1 : k + 9]]
            for prefix in (
                ["crate", "::", "dictation_shadow", "::", "enabled", "(", ")", "{"],
                ["dictation_shadow", "::", "enabled", "(", ")", "{"],
            ):
                if head[: len(prefix)] == prefix:
                    depth, close = 0, None
                    for j in range(k + 1 + len(prefix) - 1, n):
                        depth += {"{": 1, "}": -1}.get(tokens[j][0], 0)
                        if depth == 0:
                            close = j
                            break
                    if close is not None and not (close + 1 < n and tokens[close + 1][0] == "else"):
                        put(k, close + 1, "macro")
                    break
    return marks, attributes


def _shadow_lines(tokens: list[tuple[str, int]], marks: list[str | None], lines: set[int]) -> list[int]:
    """The lines of `lines` that hold shadow text outside a twin: a token under a shadow gate,
    or one that names the shadow (`dictation_shadow`, `shadow_site`)."""
    found: set[int] = set()
    for k, (text, line) in enumerate(tokens):
        if line not in lines or marks[k] in ("twin", "orig"):
            continue
        if marks[k] in ("feature", "mixed", "macro") or text in _SHADOW_IDENTS:
            found.add(line)
    return sorted(found)


def is_shadow_change(
    root: Path,
    base: str,
    guarded: list[str],
    base_table: dict[str, list[dict]] | None = None,
) -> list[str]:
    """Why this PR is a shadow change (C2): it adds or removes lines under a
    `dictation-shadow` gate, or that name the shadow, in a production file, outside the twins
    (a twin is checked by its own rule). Empty for any other PR, which may edit and delete
    live code like any change. Each reason is `path:line`."""
    reasons: list[str] = []
    for path in guarded:
        head_text = show(root, "HEAD", path)
        head = tokenize(head_text)
        marks, _ = classify(head, head_text.split("\n"), path)
        reasons.extend(f"{path}:{line}" for line in _shadow_lines(head, marks, added_lines(root, base, path)))
        old_text = show(root, base, path)
        old = tokenize(old_text)
        if old:
            old_marks, _ = classify(old, old_text.split("\n"), path, base_table)
            reasons.extend(f"{path}:{line} (removed)" for line in _shadow_lines(old, old_marks, removed_lines(root, base, path)))
    return reasons


def retired_wrappers(
    old_table: dict[str, list[dict]] | None,
    table: dict[str, list[dict]] | None = None,
) -> tuple[set[str], set[str]]:
    """The wrapper commands of the handler entries this PR retired, and the imports those
    entries list for removal with them.

    A retired entry is a `handler` entry the base table had and this one lacks. Its wrapper is
    named by the last path segment of its `twin` (`...::process_audio_witnessed,` ->
    `process_audio_witnessed`). Its `retire_imports` are the exact `use` statements, written
    out by a reviewer, that only that wrapper needed; each is normalised to its tokens. Both
    come from the **base** table: a reviewer accepted them before the cutover."""
    table = SHADOW_TWINS if table is None else table
    if not old_table:
        return set(), set()
    kept = {(path, e["first"]) for path, entries in table.items() for e in entries if e.get("form") == "handler"}
    names: set[str] = set()
    imports: set[str] = set()
    for path, entries in old_table.items():
        for entry in entries:
            if entry.get("form") != "handler" or (path, entry["first"]) in kept:
                continue
            match = re.search(r"(\w+)\s*,?\s*$", entry.get("twin", ""))
            if match:
                names.add(match.group(1))
            listed = entry.get("retire_imports", [])
            if isinstance(listed, list):
                imports.update(normalised_use(text) for text in listed if isinstance(text, str))
    return names, imports


def normalised_use(text: str) -> str:
    """A `use` statement as its tokens, joined (whitespace, comments and line breaks gone)."""
    return "".join(token for token, _ in tokenize(text))


def attribute_starts_of(tokens: list[tuple[str, int]]) -> dict[int, int]:
    """`end index -> start index` of every attribute, so the attributes in front of an item
    can be walked back over."""
    starts: dict[int, int] = {}
    at = 0
    while at < len(tokens):
        end = attribute_at(tokens, at)
        if end is None:
            at += 1
            continue
        starts[end] = at
        at = end
    return starts


_ITEM_QUALIFIERS = {"async", "unsafe", "const"}


def item_start(tokens: list[tuple[str, int]], keyword: int, attributes: dict[int, int]) -> int:
    """The index of the first token of the item whose keyword (`fn`, `use`) is at `keyword`:
    its qualifiers, its visibility (`pub`, `pub(crate)`, `pub(in path)`) and its attributes
    all belong to it, so an item is compared whole."""
    start = keyword
    while start > 0 and tokens[start - 1][0] in _ITEM_QUALIFIERS:
        start -= 1
    if start > 0 and tokens[start - 1][0] == ")":
        depth = 0
        for j in range(start - 1, -1, -1):
            depth += {")": 1, "(": -1}.get(tokens[j][0], 0)
            if depth == 0:
                if j > 0 and tokens[j - 1][0] == "pub":
                    start = j - 1
                break
    elif start > 0 and tokens[start - 1][0] == "pub":
        start -= 1
    # Outer attributes belong to the item; an inner attribute (`#![..]`) above it is the
    # module's, not the item's, and stays when the item goes.
    while start in attributes and tokens[attributes[start] + 1][0] != "!":
        start = attributes[start]
    return start


def function_item(tokens: list[tuple[str, int]], name: str) -> range | None:
    """The token indexes of `fn <name>` as a whole item: attributes, visibility, signature and
    body, up to its closing brace."""
    attributes = attribute_starts_of(tokens)
    for k in range(len(tokens) - 1):
        if tokens[k][0] != "fn" or tokens[k + 1][0] != name:
            continue
        depth, started = 0, False
        for j in range(k, len(tokens)):
            if tokens[j][0] == "{":
                depth += 1
                started = True
            elif tokens[j][0] == "}":
                depth -= 1
            if started and depth == 0:
                return range(item_start(tokens, k, attributes), j + 1)
        return None
    return None


def module_use_items(tokens: list[tuple[str, int]]) -> list[tuple[range, str]]:
    """The module-level (brace depth 0) `use` items of a text, each whole (attributes,
    visibility, `use`, the path, the closing `;`): its token range and its tokens joined. A
    `pub(crate) use` is a different item from the `use` of the same path."""
    attributes = attribute_starts_of(tokens)
    items = []
    depth = 0
    for k, (text, _) in enumerate(tokens):
        if text == "{":
            depth += 1
        elif text == "}":
            depth -= 1
        if text != "use" or depth != 0:
            continue
        end = next((j for j in range(k, len(tokens)) if tokens[j][0] == ";"), None)
        if end is None:
            continue
        span = range(item_start(tokens, k, attributes), end + 1)
        items.append((span, "".join(t for t, _ in tokens[span.start : span.stop])))
    return items


def lexical_layout(text: str) -> tuple[dict[int, int], list[dict]]:
    """Where a token or a comment spans several lines: `interior` maps each later line of a
    multi-line token (a string, a raw string) to that token's index, so an interior line is
    attributed to its token; `comments` lists every comment with its first and last line, the
    index of the token after it, and whether it is a `//` line comment."""
    interior: dict[int, int] = {}
    comments: list[dict] = []
    count, line, at = 0, 1, 0
    while at < len(text):
        match = _TOKEN.match(text, at)
        if match is None:
            line += text[at] == "\n"
            at += 1
            continue
        chunk = match.group()
        extra = chunk.count("\n")
        if match.lastgroup == "comment":
            comments.append({"first": line, "last": line + extra, "next": count, "line": chunk.startswith("//")})
        else:
            for later in range(line + 1, line + extra + 1):
                interior[later] = count
            count += 1
        line += extra
        at = match.end()
    return interior, comments


def only_retired_items_go(
    tokens: list[tuple[str, int]],
    removed: set[int],
    wrappers: list[range],
    listed: set[str],
    layout: tuple[dict[int, int], list[dict]] | None = None,
    lines: list[str] | None = None,
) -> bool:
    """Whether every removed line belongs to a retired wrapper or to a whole module-level
    `use` item a retired entry lists in `retire_imports` (compared as complete token
    sequences, visibility and attributes included).

    A line with tokens is exempt only when **every** token on it is: an `impl` that shares the
    import's line, a `pub(crate) use` of an approved path, a trait import, a glob, a local
    `use` are all shadow edits. An interior line of a multi-line string goes with its token. A
    line with no token of its own is exempt only when it is blank, a `//` comment line, or a
    line of a block comment that lies wholly inside a removed wrapper and is itself removed
    whole: deleting only the `/*` and `*/` lines of a retained comment, which makes the
    commented code live, or an interior line of a retained string, is a shadow edit. Nothing
    is inferred: whether an import is exclusive to a wrapper cannot be proved from text."""
    interior, comments = layout if layout is not None else ({}, [])
    approved = [span for span, text in module_use_items(tokens) if text in listed]
    allowed: set[int] = set()
    for span in (*wrappers, *approved):
        allowed.update(span)
    by_line: dict[int, list[int]] = {}
    for index, (_, line) in enumerate(tokens):
        by_line.setdefault(line, []).append(index)
    for line, index in interior.items():
        by_line.setdefault(line, []).append(index)
    for line in removed:
        owned = by_line.get(line, [])
        if owned:
            if any(index not in allowed for index in owned):
                return False
            continue
        if lines is not None and 0 < line <= len(lines) and not lines[line - 1].strip():
            continue
        comment = next((c for c in comments if c["first"] <= line <= c["last"]), None)
        if comment is None:
            # No token, no blank, no comment (a line this text does not hold): not exempt.
            return False
        if comment["line"]:
            continue
        before, after = comment["next"] - 1, comment["next"]
        if not set(range(comment["first"], comment["last"] + 1)) <= removed:
            return False
        if before not in allowed or after not in allowed:
            return False
    # An approved import goes whole or not at all.
    for span in approved:
        span_lines = {tokens[index][1] for index in span}
        if span_lines & removed and not span_lines <= removed:
            return False
    return True


def is_wrapper_retirement(root: Path, base: str, path: str, names: set[str], imports: set[str]) -> bool:
    """A feature-only module's change that only takes out the wrappers of retired handler
    entries: no line is added, every token on a removed line belongs to such a wrapper (its
    attributes, signature and body) or to a whole `use` item the retired entry lists in
    `retire_imports`, and the wrapper's name is gone from the whole tree (its handler entry
    and call sites went with it). Anything else in the module, any other `use` included, is a
    shadow edit."""
    if not names or added_lines(root, base, path):
        return False
    removed = removed_lines(root, base, path)
    if not removed:
        return False
    text = show(root, base, path)
    tokens = tokenize(text)
    wrappers = []
    removed_names = set()
    for name in names:
        span = function_item(tokens, name)
        if span is not None:
            wrappers.append(span)
            removed_names.add(name)
    if not removed_names:
        return False
    if not only_retired_items_go(tokens, removed, wrappers, imports, lexical_layout(text), text.split("\n")):
        return False
    for name in removed_names:
        result = subprocess.run(
            ["git", "grep", "-lw", "-e", name, "HEAD", "--", f"{TAURI}/*.rs"],
            cwd=root,
            capture_output=True,
            text=True,
        )
        if result.stdout.strip():
            return False
    return True


def shadow_module_changes(
    stats: dict[str, tuple[int, int]],
    root: Path,
    base: str,
    old_table: dict[str, list[dict]] | None = None,
) -> list[str]:
    """A change to a feature-only module (`SHADOW_ONLY`) is itself a shadow change: the PR is
    classified as one, so an ungated live edit beside it (an `instrument_invoke_handler`
    `return false`, say) is held to insertion only and feature scoping. The module's own lines
    stay exempt from insertion only (it is not in `guarded`); tests of it do not count.

    The one exception is a cutover's: removing the wrappers of handler entries the PR retires
    from the table (with their entries and every reference) is part of the retirement, not a
    shadow edit, and must not subject the cutover's deletions to insertion-only."""
    retired, imports = retired_wrappers(old_table)
    reasons = []
    for path in sorted(stats):
        if not is_production_path(path) or not is_shadow_only(path):
            continue
        if is_test_file(path, show(root, base, path) or show(root, "HEAD", path)):
            continue
        if retired and is_wrapper_retirement(root, base, path, retired, imports):
            continue
        reasons.append(f"{path} (feature-only module changed)")
    return reasons


def retired_pin_problems(
    root: Path,
    old_table: dict[str, list[dict]] | None,
    table: dict[str, list[dict]] | None = None,
) -> list[str]:
    """A pinned copy leaves the table only with the copy itself (a complete retirement).

    Every `frozen_fn` entry the base table had and this one lacks names a copy
    (`*_observed`): that function and every call site of it must be gone from the tree.
    Otherwise the protection was dropped while the live copy stays in use, and the armed and
    disarmed paths can drift apart unchecked."""
    table = SHADOW_TWINS if table is None else table
    if not old_table:
        return []
    kept = {(path, e["first"]) for path, entries in table.items() for e in entries if e.get("form") == "frozen_fn"}
    found = []
    for path, entries in old_table.items():
        for entry in entries:
            if entry.get("form") != "frozen_fn" or (path, entry["first"]) in kept:
                continue
            match = re.search(r"\bfn\s+(\w+)", entry["first"])
            if match is None:
                continue
            name = match.group(1)
            result = subprocess.run(
                ["git", "grep", "-nw", "-e", name, "HEAD", "--", f"{TAURI}/*.rs"],
                cwd=root,
                capture_output=True,
                text=True,
            )
            if result.stdout.strip():
                first = result.stdout.splitlines()[0].split(":", 1)[1]
                found.append(
                    f"{path}: the pin of `{name}` was removed from SHADOW_TWINS, but the copy or a "
                    f"call site remains (`{first[:120]}`): a pin goes only with its copy and every "
                    "call site (a complete retirement)"
                )
    return found


def first_line_of(text_lines: list[str], line: int) -> str:
    return text_lines[line - 1].strip() if 0 < line <= len(text_lines) else ""


def function_tokens(
    tokens: list[tuple[str, int]], lines: list[str], first: str
) -> list[str] | None:
    """The normalised tokens of the function whose first line is `first`, or `None`."""
    for k, (_, line) in enumerate(tokens):
        if (k == 0 or tokens[k - 1][1] != line) and first_line_of(lines, line) == first:
            return normalised([t for t, _ in tokens[k : statement_end(tokens, k)]])
    return None


def twin_problems(
    root: Path,
    base: str,
    path: str,
    allow: dict[str, list[dict]] | None = None,
) -> list[str]:
    """Twin consistency and no new twins, on every PR (C2 ii, iii; C3).

    * Every `#[cfg(not(feature = "dictation-shadow"))]` of the file names a statement in
      `SHADOW_TWINS` (no new twins), and the feature twin beside it must be its original plus
      nothing or one permitted insertion (`closed`), the pair a reviewer pinned (`frozen`), or
      a `generate_handler!` entry's wrapper (`handler`). An original that changes needs its
      twin to change with it, and a frozen pair needs a new reviewed digest.
    * Every `frozen_fn` entry pins a shadow copy of a live function (`*_observed`) against its
      original by digest: either side changing fails until a reviewer pins the new one.
    * Any other inserted `cfg` that switches an item off in a feature build is a new twin.
    """
    allow = SHADOW_TWINS if allow is None else allow
    entries = [dict(entry) for entry in allow.get(path, [])]
    pending = [e for e in entries if e["form"] != "frozen_fn"]
    pins = [e for e in entries if e["form"] == "frozen_fn"]
    head = show(root, "HEAD", path)
    lines = head.split("\n")
    tokens = tokenize(head)
    base_text = show(root, base, path)
    base_tokens = tokenize(base_text)
    base_lines = base_text.split("\n")
    added = added_lines(root, base, path)
    regions = twin_regions(tokens)
    old_regions = twin_regions(base_tokens)
    old_list = [old_regions[start] for start in sorted(old_regions)]
    found: list[str] = []
    for position, start in enumerate(sorted(regions)):
        region = regions[start]
        line = tokens[region["gate"]][1]
        target = tokens[start][1]
        statement = first_line_of(lines, target)
        entry = next((e for e in pending if e["first"] == statement and not e.get("used")), None)
        if entry is None:
            found.append(
                f"{path}:{line}: `{NEGATED_GATE}` disables the existing statement at line "
                f"{target} in the feature build, and `{statement}` is not in SHADOW_TWINS "
                "(no new twins: a twin is added only by a reviewed SHADOW_TWINS entry)"
            )
            continue
        entry["used"] = True
        # The base's pair at the same place, to say which side changed: only when the PR
        # added and removed no pair, so that places line up.
        was = old_list[position] if len(old_list) == len(regions) else None
        form = entry["form"]
        if form == "handler":
            if region["twin_stack"] or region["original_stack"]:
                found.append(f"{path}:{line}: the handler entry at line {target} carries another attribute")
            # Both paths are pinned: the entry is the command itself and the feature entry is
            # its witnessing wrapper, until the pair is retired (the gates, the wrapper entry
            # and the table entry all go together). A feature entry that names the command
            # itself, or another wrapper, skips the reply witness.
            want_original = normalised([t for t, _ in tokenize(entry["first"])])
            want_twin = normalised([t for t, _ in tokenize(entry.get("twin", ""))])
            if region["original"] != want_original:
                found.append(f"{path}:{line}: the handler entry at line {target} is not `{entry['first']}`")
            if region["twin"] is None:
                found.append(
                    f"{path}:{line}: the handler entry `{entry['first']}` has no feature entry: "
                    f"the shadow build must register `{entry.get('twin', '')}`"
                )
            elif region["twin"] != want_twin:
                found.append(
                    f"{path}:{line}: the feature handler entry of `{entry['first']}` is not "
                    f"`{entry.get('twin', '')}`: the shadow build must register the witnessing "
                    "wrapper, not another command"
                )
            continue
        if region["twin"] is None:
            found.append(f"{path}:{line}: the twin of the statement at line {target} is missing")
            continue
        if form == "frozen":
            digest = pair_hash(region["original"], region["twin"])
            if digest != entry["pair"]:
                found.append(
                    f"{path}:{line}: the frozen twin pair at line {target} changed "
                    f"({digest}, reviewed {entry['pair']}): it is not the pair a reviewer accepted. "
                    + _which_changed(region, was)
                    + " After a reviewer confirms the twin matches its original, pin the new "
                    "digest in SHADOW_TWINS."
                )
        else:
            difference = twin_difference(region["original"], region["twin"])
            if difference is not None:
                found.append(
                    f"{path}:{line}: the feature twin of the statement at line {target} "
                    f"differs from the original: {difference}. " + _which_changed(region, was)
                )
    for entry in pending:
        if not entry.get("used"):
            found.append(
                f"{path}: SHADOW_TWINS names `{entry['first']}`, which has no "
                f"`{NEGATED_GATE}` twin in the file any more (an entry goes with its twin)"
            )
    for entry in pins:
        name = entry["first"]
        original = function_tokens(tokens, lines, entry["original"])
        observed = function_tokens(tokens, lines, name)
        if original is None or observed is None:
            missing = entry["original"] if original is None else name
            found.append(f"{path}: SHADOW_TWINS pins `{name}` against `{entry['original']}`, but `{missing}` is not in the file")
            continue
        digest = pair_hash(original, observed)
        if digest == entry["pair"]:
            continue
        was_original = function_tokens(base_tokens, base_lines, entry["original"])
        was_observed = function_tokens(base_tokens, base_lines, name)
        if was_original is not None and was_observed is not None:
            if original != was_original and observed == was_observed:
                hint = "the original changed but its observed copy did not: change both."
            elif observed != was_observed and original == was_original:
                hint = "the observed copy changed but its original did not."
            else:
                hint = "both changed."
        else:
            hint = ""
        found.append(
            f"{path}: the observed copy `{name}` is no longer the pinned copy of "
            f"`{entry['original']}` ({digest}, reviewed {entry['pair']}); {hint} After a "
            "reviewer confirms the copy still matches its original, apart from the shadow's "
            "own observation, pin the new digest in SHADOW_TWINS."
        )
    # Any other inserted attribute that switches an item off in a feature build is a twin of
    # a kind the table does not name: `any(not(..), ..)`, a raw string, `cfg_attr(.., cfg(..))`.
    at = 0
    while at < len(tokens):
        end = attribute_at(tokens, at)
        if end is None:
            at += 1
            continue
        compact = "".join(t for t, _ in tokens[at:end])
        if (
            tokens[at][1] in added
            and re.match(r"#!?\[(cfg|cfg_attr)\b", compact)
            and compact != NEGATED_GATE
            and _negates_shadow(tokens, at, end)
        ):
            found.append(
                f"{path}:{tokens[at][1]}: `{compact}` switches an item off in the feature "
                f"build: no new twins; the only such gate is `{NEGATED_GATE}` on a SHADOW_TWINS entry"
            )
        at = end
    return found


def _which_changed(region: dict, was: dict | None) -> str:
    """Which side of a twin the branch changed, against the base's pair, for the message."""
    if was is None:
        return ""
    original = region["original"] != was["original"]
    twin = region["twin"] != was["twin"]
    if original and not twin:
        return "The original changed but its twin did not: an original and its twin change together."
    if twin and not original:
        return "The twin changed but its original did not."
    if original and twin:
        return "Both changed."
    return ""


def scoping_problems(
    root: Path,
    base: str,
    path: str,
) -> list[str]:
    """What a shadow change may not do to a production file (D-S2c-8 a, D-S2c-12 c; C2 i).

    * Every inserted executable token is behind the feature: under a `dictation-shadow` gate,
      in a feature twin, in a test-only statement, a `shadow_site!` call or an
      `if dictation_shadow::enabled() { .. }` block with no `else`. An inserted early
      `return` with no gate is not.
    * An inserted `cfg` or `cfg_attr` on an existing statement, or at file level in a file
      the base had, is rejected without being evaluated, and so is every other attribute but
      the named `SHADOW_ATTR_ALLOW` entries.
    * The base's tokens survive, in order, once the gated insertions are taken out."""
    attrs_pending = [dict(entry) for entry in SHADOW_ATTR_ALLOW.get(path, [])]
    head = show(root, "HEAD", path)
    lines = head.split("\n")
    tokens = tokenize(head)
    added = added_lines(root, base, path)
    in_base = bool(show(root, base, path))
    regions = twin_regions(tokens)
    marks, attributes = classify(tokens, lines, path)
    found: list[str] = []
    # A new file adds nothing to the live path by itself: the `mod` line that includes it is
    # an inserted statement of its parent and is checked there. A file of `SHADOW_DIFF_ALLOW`
    # is held to the same rules: the allowance is for the named removed line alone.
    exempt: set[int] = set()
    allowed = SHADOW_DIFF_ALLOW.get(path)
    if allowed is not None:
        base_text_lines = show(root, base, path).split("\n")
        permitted_text = allowed[2] if len(allowed) > 2 else None
        for number in sorted(removed_lines(root, base, path)):
            text = base_text_lines[number - 1].strip() if 0 < number <= len(base_text_lines) else ""
            if permitted_text is not None and text not in permitted_text:
                found.append(
                    f"{path}:{number}: removes `{text}`, which is not the line SHADOW_DIFF_ALLOW "
                    "names: the allowance covers that one line only"
                )
            else:
                exempt.add(number)
    if in_base:
        for entry in SHADOW_UNGATED_ALLOW.get(path, []):
            for k, (_, line) in enumerate(tokens):
                if (k == 0 or tokens[k - 1][1] != line) and first_line_of(lines, line) == entry["first"]:
                    for j in range(k, statement_end(tokens, k)):
                        marks[j] = marks[j] or "named"
                    break
        reported: set[int] = set()
        for k, (text, line) in enumerate(tokens):
            if line not in added or k in attributes or line in reported:
                continue
            if marks[k] in ("feature", "test", "twin", "macro", "named"):
                continue
            reported.add(line)
            found.append(
                f"{path}:{line}: inserted code is not behind the dictation-shadow feature "
                f"(`{first_line_of(lines, line)}`): in a shadow change every inserted "
                "statement sits under `#[cfg(feature = \"dictation-shadow\")]`"
            )
    at = 0
    while at < len(tokens):
        end = attribute_at(tokens, at)
        if end is None:
            at += 1
            continue
        line = tokens[at][1]
        compact = "".join(t for t, _ in tokens[at:end])
        gate = re.match(r"#(!?)\[", compact)
        if gate is None or line not in added:
            at = end
            continue
        is_cfg = re.match(r"#!?\[(cfg|cfg_attr)\b", compact) is not None
        if gate.group(1):
            if in_base:
                found.append(f"{path}:{line}: `{compact}` gates the whole of a file the base already had")
            at = end
            continue
        # The statement or item the group of attributes gates.
        start = end
        while (nxt := attribute_at(tokens, start)) is not None:
            start = nxt
        if start >= len(tokens):
            at = end
            continue  # the end of the file
        negated = compact == NEGATED_GATE
        # A feature twin's own gate follows its original statement and is checked with that
        # statement's gate (`twin_problems`). Whether a line is "added" is a line diff's
        # reading, which is ambiguous when a twin repeats its original's lines, so a negated
        # gate is never skipped for gating text that reads as new, and a twin's gate is never
        # read as sitting on an existing statement.
        if compact == FEATURE_GATE and any(r["twin_attr"] == at for r in regions.values()):
            at = end
            continue
        if not negated and tokens[start][1] in added:
            at = end
            continue  # new text
        target = tokens[start][1]
        statement = first_line_of(lines, target)
        if not is_cfg:
            named = next(
                (
                    e
                    for e in attrs_pending
                    if e["attr"] == compact and e["first"] == statement and not e.get("used")
                ),
                None,
            )
            if named is None:
                found.append(
                    f"{path}:{line}: `{compact}` is inserted on the existing statement at line "
                    f"{target}; no attribute may be, except the named SHADOW_ATTR_ALLOW entries"
                )
            else:
                named["used"] = True
            at = end
            continue
        if compact != NEGATED_GATE:
            found.append(
                f"{path}:{line}: `{compact}` on the existing statement at line {target} is not "
                f"the one permitted gate, `{NEGATED_GATE}` on an allowlisted twin or handler entry"
            )
        at = end
    # The base's tokens survive, except the one line a `SHADOW_DIFF_ALLOW` entry names.
    if in_base:
        found.extend(base_survives(root, base, path, tokens, added, exempt))
    return found


def twin_table_problems(allow: dict[str, list[dict]] | None = None) -> list[str]:
    """What is wrong with the allow list itself: an entry with no reason, an unknown form, a
    frozen entry with no digest, a pinned function with no original, a file that is not
    production code."""
    allow = SHADOW_TWINS if allow is None else allow
    found = []
    for path, entries in allow.items():
        if not is_production_path(path):
            found.append(f"SHADOW_TWINS[{path}] names a file that is not a production file")
        for entry in entries:
            name = f"SHADOW_TWINS[{path}][{entry.get('first')!r}]"
            if entry.get("form") not in ("handler", "closed", "frozen", "frozen_fn"):
                found.append(f"{name} has no known form")
            if not str(entry.get("why", "")).strip():
                found.append(f"{name} gives no reason")
            if entry.get("form") in ("frozen", "frozen_fn") and not entry.get("pair"):
                found.append(f"{name} is frozen without a digest")
            if entry.get("form") == "frozen_fn" and not str(entry.get("original", "")).strip():
                found.append(f"{name} pins a function without naming its original")
            if entry.get("form") == "handler" and not str(entry.get("twin", "")).strip():
                found.append(f"{name} is a handler entry without its witnessing wrapper (`twin`)")
            if "retire_imports" in entry:
                imports = entry["retire_imports"]
                if entry.get("form") != "handler":
                    found.append(f"{name} lists retire_imports, which only a handler entry may")
                if not isinstance(imports, list) or not all(isinstance(i, str) for i in imports):
                    found.append(f"{name} retire_imports must be a list of strings, not {imports!r}")
                    imports = []
                for statement in imports:
                    parsed = tokenize(statement)
                    items = module_use_items(parsed)
                    if len(items) != 1 or items[0][0] != range(len(parsed)) or "*" in [t for t, _ in parsed]:
                        found.append(f"{name} retire_imports {statement!r} is not exactly one whole `use ...;` item without a glob")
    return found


def allow_table_problems(allow: dict[str, tuple] | None = None) -> list[str]:
    """What is wrong with the list of named removals itself."""
    allow = SHADOW_DIFF_ALLOW if allow is None else allow
    found = []
    for path, entry in allow.items():
        reason = entry[1]
        if not reason.strip():
            found.append(f"SHADOW_DIFF_ALLOW[{path}] gives no reason")
        if not is_production_path(path):
            found.append(f"SHADOW_DIFF_ALLOW[{path}] names a file that is not a production file")
    for path, entries in SHADOW_UNGATED_ALLOW.items():
        if not is_production_path(path):
            found.append(f"SHADOW_UNGATED_ALLOW[{path}] names a file that is not a production file")
        for entry in entries:
            if not str(entry.get("why", "")).strip():
                found.append(f"SHADOW_UNGATED_ALLOW[{path}][{entry.get('first')!r}] gives no reason")
    return found


def violations(
    stats: dict[str, tuple[int, int]],
    guarded: list[str] | None = None,
    allow: dict[str, tuple[int, str]] | None = None,
) -> list[str]:
    """The insertion-only rule: no production file of a shadow change loses a line."""
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
    found.extend(allow_table_problems(allow))
    return found


def resolve_base(root: Path, base: str) -> str:
    """The merge base of `base` and `HEAD`, as a commit hash, resolved once. Every diff
    coordinate (added and removed line numbers) and every base-side source read of one run
    then uses this one revision: if the base branch advances between two reads, a line number
    taken from the diff would otherwise be looked up in a text that has moved."""
    result = subprocess.run(["git", "merge-base", base, "HEAD"], cwd=root, capture_output=True, text=True)
    if result.returncode != 0 or not result.stdout.strip():
        raise RuntimeError(f"git merge-base {base} HEAD failed: {result.stderr.strip()}")
    return result.stdout.strip()


def base_twin_table(root: Path, base: str) -> dict[str, list[dict]] | None:
    """`SHADOW_TWINS` as this script was at the base revision, or `None` when the base has no
    such table (then the running script's own is the reference). The base's text is classified
    with the base's table: a PR that retires a copy removes the entry and the code together,
    and the base's copy is still the twin the table named."""
    text = show(root, base, "scripts/check_shadow_diff.py")
    if not text:
        return None
    try:
        tree = ast.parse(text)
        for node in tree.body:
            target = node.target if isinstance(node, ast.AnnAssign) else (node.targets[0] if isinstance(node, ast.Assign) else None)
            if isinstance(target, ast.Name) and target.id == "SHADOW_TWINS" and node.value is not None:
                return ast.literal_eval(node.value)
    except (SyntaxError, ValueError):
        return None
    return None


def main(root: Path = ROOT, base: str = "origin/main") -> int:
    try:
        base = resolve_base(root, base)
        stats = numstat(root, base)
    except RuntimeError as error:
        print(error, file=sys.stderr)
        return 2
    old_table = base_twin_table(root, base)
    guarded = guarded_paths(stats, root, base)
    # On every PR (C2 ii, iii; C3): twins stay consistent, pinned copies stay pinned, and no
    # new twin appears.
    problems = twin_table_problems()
    for path in guarded:
        problems.extend(twin_problems(root, base, path))
    # Only a PR that adds or removes shadow lines is a shadow change, and only it is held to
    # insertion only, with every inserted statement behind the feature (C2 i). Any other PR
    # may edit and delete live code, with its originals and twins kept consistent above.
    reasons = is_shadow_change(root, base, guarded, old_table) + shadow_module_changes(stats, root, base, old_table)
    problems.extend(retired_pin_problems(root, old_table))
    if reasons:
        problems.extend(violations(stats, guarded))
        for path in guarded:
            problems.extend(scoping_problems(root, base, path))
    else:
        problems.extend(allow_table_problems())
    for path in guarded:
        added, removed = stats[path]
        print(f"{added}\t{removed}\t{path}")
    if reasons:
        print(f"shadow change: yes ({len(reasons)} shadow lines, first {reasons[0]})")
    else:
        print("shadow change: no (insertion-only and feature scoping do not apply)")
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
