#!/usr/bin/env python3
"""List every temporary `hunt!` log site and fail if one has no issue number.

Hunting logs are `hunt!(issue = <number>, ...)` calls in the Rust sources. They
emit at debug level, stay local, and are removed when their issue closes. This
check only reads the sources: it never calls the GitHub API, so it cannot tell
whether an issue is still open. The listing is the reminder to remove them.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[1]
SOURCE_DIR = Path("app/src-tauri/src")

# `hunt!(`, `hunt!{` and `hunt![` all invoke the macro.
SITE = re.compile(r"\bhunt!\s*[(\[{]")
BLOCK_COMMENT = re.compile(r"/\*.*?\*/", re.S)
LINE_COMMENT = re.compile(r"//[^\n]*")
# Everything from a `#[cfg(test)] mod` to the end of the file is test code.
TEST_MODULE = re.compile(r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+")
MACRO_DEFINITION = re.compile(r"macro_rules!\s+hunt\s*\{")
ISSUE = re.compile(r"\s*issue\s*=\s*(\d+)\s*,")
# Emitting on the hunt target directly would skip the macro's issue requirement.
DIRECT_TARGET = re.compile(r"""target:\s*"hunt\"""")


class Site(NamedTuple):
    path: str
    line: int
    issue: int | None


def _line_number(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def _blank(match: re.Match) -> str:
    """Replace matched text with spaces, keeping newlines so line numbers hold."""
    return re.sub(r"[^\n]", " ", match.group(0))


def _blank_macro_definition(text: str) -> str:
    """Blank `macro_rules! hunt { ... }`, whose body names the target itself."""
    match = MACRO_DEFINITION.search(text)
    if match is None:
        return text
    depth, end = 1, match.end()
    while end < len(text) and depth:
        depth += {"{": 1, "}": -1}.get(text[end], 0)
        end += 1
    blanked = re.sub(r"[^\n]", " ", text[match.start() : end])
    return text[: match.start()] + blanked + text[end:]


def code_only(text: str) -> str:
    """The source with comments, the macro's own definition and test modules
    blanked, so doc examples and fixtures are not counted as sites."""
    test_module = TEST_MODULE.search(text)
    if test_module is not None:
        text = text[: test_module.start()]
    text = BLOCK_COMMENT.sub(_blank, text)
    text = LINE_COMMENT.sub(_blank, text)
    return _blank_macro_definition(text)


def scan(root: Path) -> tuple[list[Site], list[str]]:
    """Return every hunt site and a list of other problems."""
    sites: list[Site] = []
    problems: list[str] = []
    for path in sorted((root / SOURCE_DIR).rglob("*.rs")):
        relative = path.relative_to(root)
        text = code_only(path.read_text(encoding="utf-8", errors="replace"))
        for match in SITE.finditer(text):
            issue = ISSUE.match(text, match.end())
            sites.append(
                Site(
                    str(relative),
                    _line_number(text, match.start()),
                    int(issue.group(1)) if issue else None,
                )
            )
        for match in DIRECT_TARGET.finditer(text):
            problems.append(
                f'{relative}:{_line_number(text, match.start())}: '
                'target: "hunt" bypasses hunt!; use hunt!(issue = <number>, ...)'
            )
    return sites, problems


def main(root: Path = ROOT) -> int:
    sites, problems = scan(root)
    print(f"hunt sites: {len(sites)}")
    for site in sites:
        label = f"issue #{site.issue}" if site.issue is not None else "NO ISSUE NUMBER"
        print(f"  {site.path}:{site.line}  {label}")
    for site in sites:
        if site.issue is None:
            problems.append(
                f"{site.path}:{site.line}: hunt! needs `issue = <number>,` as its first argument"
            )
    for problem in problems:
        print(f"error: {problem}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
