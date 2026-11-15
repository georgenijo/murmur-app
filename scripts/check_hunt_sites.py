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

SITE = re.compile(r"\bhunt!\s*\(")
ISSUE = re.compile(r"\s*issue\s*=\s*(\d+)\s*,")
# Emitting on the hunt target directly would skip the macro's issue requirement.
DIRECT_TARGET = re.compile(r"""target:\s*"hunt\"""")
# The macro lives here; its own definition, docs and test fixtures are not hunt sites.
MACRO_HOME = Path("app/src-tauri/src/telemetry.rs")


class Site(NamedTuple):
    path: str
    line: int
    issue: int | None


def _line_number(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def _is_comment(text: str, offset: int) -> bool:
    start = text.rfind("\n", 0, offset) + 1
    return text[start:offset].lstrip().startswith("//")


def scan(root: Path) -> tuple[list[Site], list[str]]:
    """Return every hunt site and a list of other problems."""
    sites: list[Site] = []
    problems: list[str] = []
    for path in sorted((root / SOURCE_DIR).rglob("*.rs")):
        relative = path.relative_to(root)
        text = path.read_text(encoding="utf-8", errors="replace")
        for match in SITE.finditer(text):
            if relative == MACRO_HOME or _is_comment(text, match.start()):
                continue
            issue = ISSUE.match(text, match.end())
            sites.append(
                Site(
                    str(relative),
                    _line_number(text, match.start()),
                    int(issue.group(1)) if issue else None,
                )
            )
        if relative != MACRO_HOME:
            for match in DIRECT_TARGET.finditer(text):
                if not _is_comment(text, match.start()):
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
