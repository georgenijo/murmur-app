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
SHADOW_TWINS: dict[str, list[dict]] = {
    "app/src-tauri/src/commands/recording.rs": [
        {"first": "fn spawn_dictation_partial_ticker(", "form": "frozen", "pair": "51754c8ecf2fce6d", "why": "the shared ticker lookup (`ticker_start_check`) replaces the spawner's head, so the skip taken is the witness of that one lookup (D-S2c-10 R2)"},
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
    ],
    "app/src-tauri/src/core_history.rs": [
        {"first": "pub(crate) fn consume<R>(&mut self, decide: impl FnOnce(OpStatus) -> R) -> R {", "form": "frozen", "pair": "dcc7056f2f3ceb6a", "why": "keeps the answer of the H1 retirement, which the original discards, for the flight's H1 witness"},
    ],
    "app/src-tauri/src/dictation_executor/executor.rs": [
        {"first": "FieldAdapter::cancel_through(ports.app_state(), id.0);", "form": "frozen", "pair": "c4e1922cc096db96", "why": 'an observing flight takes the mirror witness the cancel returns (the fetch_max result the original discards)'},
        {"first": "FieldAdapter::clear_context(ports.app_state(), id.0);", "form": "frozen", "pair": "c865ad563d1e19a2", "why": 'an observing flight takes the mirror witness, built after the leaf guard is released (D-S2c-12 a); otherwise the original'},
        {"first": "FieldAdapter::set_context(ports.app_state(), id.0, snapshot);", "form": "frozen", "pair": "1fa8e76a377b9034", "why": 'an observing flight takes the mirror witness, built after the leaf guard is released (D-S2c-12 a); otherwise the original'},
    ],
    "app/src-tauri/src/lib.rs": [
        {"first": "commands::recording::process_audio,", "form": "handler", "why": 'the wrapper command of the same name is inserted with the opposite gate and calls this one (D-S2c-8 amendment 1)'},
        {"first": "commands::recording::start_native_recording,", "form": "handler", "why": 'the wrapper command of the same name is inserted with the opposite gate and calls this one (D-S2c-8 amendment 1)'},
        {"first": "commands::recording::stop_native_recording,", "form": "handler", "why": 'the wrapper command of the same name is inserted with the opposite gate and calls this one (D-S2c-8 amendment 1)'},
        {"first": "commands::recording::cancel_native_recording,", "form": "handler", "why": 'the wrapper command of the same name is inserted with the opposite gate and calls this one (D-S2c-8 amendment 1)'},
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
            stop = statement_end(tokens, start)
            original = normalised([t for t, _ in tokens[start:stop]])
            twin: list[str] | None = None
            twin_attr: int | None = None
            cursor = stop
            twin_end = attribute_at(tokens, cursor)
            if twin_end is not None and "".join(t for t, _ in tokens[cursor:twin_end]) == FEATURE_GATE:
                twin_attr = cursor
                begin = twin_end
                while (nxt := attribute_at(tokens, begin)) is not None:
                    begin = nxt
                twin = normalised([t for t, _ in tokens[begin : statement_end(tokens, begin)]])
            found[start] = {"original": original, "twin": twin, "twin_attr": twin_attr}
        at = end
    return found


def first_line_of(text_lines: list[str], line: int) -> str:
    return text_lines[line - 1].strip() if 0 < line <= len(text_lines) else ""


def gated_originals(
    root: Path,
    base: str,
    path: str,
    allow: dict[str, list[dict]] | None = None,
) -> list[str]:
    """Inserted gates on code the base already had (D-S2c-12 b, c). Every inserted `cfg` or
    `cfg_attr` on an existing statement or item is rejected, unless it is exactly
    `#[cfg(not(feature = "dictation-shadow"))]` on a statement `SHADOW_TWINS` names, and
    then the feature twin beside it must be the original plus nothing or one permitted
    insertion (or be the frozen pair the entry names, or be a handler entry). A file-level
    `#![cfg(..)]` / `#![cfg_attr(..)]` added to a file the base had is rejected."""
    allow = SHADOW_TWINS if allow is None else allow
    pending = [dict(entry) for entry in allow.get(path, [])]
    attrs_pending = [dict(entry) for entry in SHADOW_ATTR_ALLOW.get(path, [])]
    head = show(root, "HEAD", path)
    lines = head.splitlines()
    tokens = tokenize(head)
    added = added_lines(root, base, path)
    in_base = bool(show(root, base, path))
    regions = twin_regions(tokens)
    found: list[str] = []
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
        # statement's gate. Whether a line is "added" is a line diff's reading, which is
        # ambiguous when a twin repeats its original's lines, so a negated gate is never
        # skipped for gating text that reads as new, and a twin's gate is never read as
        # sitting on an existing statement.
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
            continue
        region = regions.get(start)
        entry = next(
            (e for e in pending if e["first"] == statement and not e.get("used")), None
        )
        if entry is None or region is None:
            found.append(
                f"{path}:{line}: `{NEGATED_GATE}` disables the existing statement at line "
                f"{target} in the feature build, and `{statement}` is not in SHADOW_TWINS"
            )
            at = end
            continue
        entry["used"] = True
        form = entry["form"]
        if form == "handler":
            pass
        elif region["twin"] is None:
            found.append(f"{path}:{line}: the twin of the statement at line {target} is missing")
        elif form == "frozen":
            digest = pair_hash(region["original"], region["twin"])
            if digest != entry["pair"]:
                found.append(
                    f"{path}:{line}: the frozen twin pair at line {target} changed "
                    f"({digest}, reviewed {entry['pair']}): it is not the pair a reviewer accepted"
                )
        else:
            difference = twin_difference(region["original"], region["twin"])
            if difference is not None:
                found.append(
                    f"{path}:{line}: the feature twin of the statement at line {target} "
                    f"differs from the original: {difference}"
                )
        at = end
    return found


def twin_table_problems(allow: dict[str, list[dict]] | None = None) -> list[str]:
    """What is wrong with the allow list itself: an entry with no reason, an unknown form, a
    frozen entry with no digest, a file that is not production code."""
    allow = SHADOW_TWINS if allow is None else allow
    found = []
    for path, entries in allow.items():
        if not is_production_path(path):
            found.append(f"SHADOW_TWINS[{path}] names a file that is not a production file")
        for entry in entries:
            name = f"SHADOW_TWINS[{path}][{entry.get('first')!r}]"
            if entry.get("form") not in ("handler", "closed", "frozen"):
                found.append(f"{name} has no known form")
            if not str(entry.get("why", "")).strip():
                found.append(f"{name} gives no reason")
            if entry.get("form") == "frozen" and not entry.get("pair"):
                found.append(f"{name} is frozen without a digest")
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
        problems.extend(gated_originals(root, base, path))
    problems.extend(twin_table_problems())
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
