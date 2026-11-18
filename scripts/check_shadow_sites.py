#!/usr/bin/env python3
"""List every `shadow_site!` capture site and fail if one is malformed.

A dictation shadow capture site is a `shadow_site!(issue = <number>)` marker next
to the code that feeds the shadow recorder (appendix C8.4 step 6). The marker
expands to nothing in a build without the `dictation-shadow` feature. Every site
names the issue that tracks removing shadow mode, so the listing is the removal
checklist: when that issue closes, delete each site it prints. This check only
reads the sources: it never calls the GitHub API, so it cannot tell whether the
issue is still open.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path
from typing import NamedTuple

ROOT = Path(__file__).resolve().parents[1]
SOURCE_DIR = Path("app/src-tauri/src")

# `shadow_site!(`, `shadow_site!{` and `shadow_site![` all invoke the macro.
# Rust allows whitespace, newlines and comments between the macro name and the
# `!`. Comments are blanked (to spaces) before this runs, so `\s*` covers them.
SITE = re.compile(r"\bshadow_site\s*!\s*[(\[{]")
ISSUE = re.compile(r"\s*issue\s*=\s*(\d+)\s*[,)\]}]")
# The one accepted shape after the opening bracket: the issue number and nothing else.
WELL_FORMED = re.compile(r"\s*issue\s*=\s*\d+\s*,?\s*[)\]}]")
TEST_MODULE = re.compile(
    r"#\[cfg\(test\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{"
)
MACRO_DEFINITION = re.compile(r"macro_rules\s*!\s*shadow_site\s*\{")
CHAR_LITERAL = re.compile(r"'(?:\\[^']+|[^\\'])'")
RAW_STRING_START = re.compile(r'r(#*)"')


class Site(NamedTuple):
    path: str
    line: int
    issue: int | None
    well_formed: bool = True


def _line_number(text: str, offset: int) -> int:
    return text.count("\n", 0, offset) + 1


def _mask(text: str) -> tuple[str, str]:
    """Return `(code, skeleton)`, both the same length as `text`.

    `code` has comments (line, and nested block) blanked; string literals are
    kept, so `//` inside a URL string is not a comment. `skeleton` also blanks
    string and character literal contents, so braces can be matched without
    being fooled by text. Newlines survive in both, keeping line numbers.
    """
    code, skeleton = list(text), list(text)

    def blank(chars: list[str], start: int, end: int) -> None:
        for k in range(start, end):
            if chars[k] != "\n":
                chars[k] = " "

    i, n = 0, len(text)
    while i < n:
        two = text[i : i + 2]
        if two == "//":
            end = text.find("\n", i)
            end = n if end == -1 else end
            blank(code, i, end)
            blank(skeleton, i, end)
            i = end
        elif two == "/*":
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            blank(code, i, j)
            blank(skeleton, i, j)
            i = j
        elif text[i] == '"' or (raw := RAW_STRING_START.match(text, i)):
            if text[i] == '"':
                j = i + 1
                while j < n and text[j] != '"':
                    j += 2 if text[j] == "\\" else 1
                j = min(j + 1, n)
            else:
                closing = '"' + raw.group(1)
                found = text.find(closing, raw.end())
                j = n if found == -1 else found + len(closing)
            blank(skeleton, i + 1, max(i + 1, j - 1))
            i = j
        elif text[i] == "'" and (char := CHAR_LITERAL.match(text, i)):
            blank(skeleton, i + 1, char.end() - 1)
            i = char.end()
        else:
            i += 1
    return "".join(code), "".join(skeleton)


def _blank_braced(code: str, skeleton: str, pattern: re.Pattern, whole: bool) -> tuple[str, str]:
    """Blank every `pattern` block (up to its matching closing brace).

    The pattern must end at the opening brace. With `whole` the match itself is
    blanked too; otherwise only the body is.
    """
    out_code, out_skeleton = list(code), list(skeleton)
    for match in pattern.finditer(skeleton):
        depth, end = 1, match.end()
        while end < len(skeleton) and depth:
            depth += {"{": 1, "}": -1}.get(skeleton[end], 0)
            end += 1
        for k in range(match.start() if whole else match.end(), end):
            if code[k] != "\n":
                out_code[k] = out_skeleton[k] = " "
    return "".join(out_code), "".join(out_skeleton)


def code_only(text: str) -> str:
    """The source with comments, the macro's own definition and the bodies of
    `#[cfg(test)]` modules blanked, so doc examples, the definition and fixtures
    are not counted as sites. Production code after a test module still counts."""
    code, skeleton = _mask(text)
    code, skeleton = _blank_braced(code, skeleton, MACRO_DEFINITION, whole=True)
    code, _ = _blank_braced(code, skeleton, TEST_MODULE, whole=False)
    return code


def scan(root: Path) -> tuple[list[Site], list[str]]:
    """Return every shadow site and a list of other problems."""
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
                    WELL_FORMED.match(text, match.end()) is not None,
                )
            )
    return sites, problems


def main(root: Path = ROOT) -> int:
    sites, problems = scan(root)
    print(f"shadow sites: {len(sites)}")
    for site in sites:
        label = f"issue #{site.issue}" if site.issue is not None else "NO ISSUE NUMBER"
        print(f"  {site.path}:{site.line}  {label}")
    for site in sites:
        if site.issue is None:
            problems.append(
                f"{site.path}:{site.line}: shadow_site! needs `issue = <number>`"
            )
        elif not site.well_formed:
            problems.append(
                f"{site.path}:{site.line}: shadow_site! takes only `issue = <number>`; "
                "it accepts no other arguments"
            )
    for problem in problems:
        print(f"error: {problem}", file=sys.stderr)
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
