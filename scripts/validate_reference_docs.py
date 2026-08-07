#!/usr/bin/env python3
"""Verify that Tauri command reference docs match the registered command API."""

from __future__ import annotations

from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
LIB_RS = ROOT / "app/src-tauri/src/lib.rs"
COMMANDS_DOC = ROOT / "docs/reference/commands.md"
COUNT_DOCS = (
    ROOT / "CLAUDE.md",
    ROOT / "AGENTS.md",
    ROOT / "docs/ARCHITECTURE.md",
)


def registered_commands(lib_rs: str) -> list[str]:
    handler = re.search(
        r"\.invoke_handler\(tauri::generate_handler!\[(?P<body>.*?)\]\)",
        lib_rs,
        re.S,
    )
    if handler is None:
        raise AssertionError("lib.rs: missing Tauri generate_handler! registration")
    body = handler.group("body")
    commands = re.findall(
        r"^\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)\s*,?\s*$",
        body,
        re.M,
    )
    nonempty_lines = [
        line for line in body.splitlines() if line.strip() and not line.lstrip().startswith("//")
    ]
    if len(commands) != len(nonempty_lines):
        raise AssertionError(
            "lib.rs: generate_handler! contains an unrecognized command entry"
        )
    if len(commands) != len(set(commands)):
        raise AssertionError("lib.rs: generate_handler! contains duplicate command names")
    return commands


def documented_commands(commands_doc: str) -> list[str]:
    commands = re.findall(
        r"^\| `([A-Za-z_][A-Za-z0-9_]*)` \|",
        commands_doc,
        re.M,
    )
    if len(commands) != len(set(commands)):
        raise AssertionError("commands.md: duplicate command rows")
    return commands


def command_count_mentions(text: str) -> list[int]:
    patterns = (
        r"\((\d+) Tauri commands\)",
        r"\b(\d+) registered commands\b",
        r"\b(\d+) commands are registered\b",
    )
    return [
        int(match)
        for pattern in patterns
        for match in re.findall(pattern, text)
    ]


def validate_reference_docs(root: Path = ROOT) -> int:
    registered = registered_commands(
        (root / LIB_RS.relative_to(ROOT)).read_text()
    )
    documented = documented_commands(
        (root / COMMANDS_DOC.relative_to(ROOT)).read_text()
    )
    registered_set = set(registered)
    documented_set = set(documented)
    if registered_set != documented_set:
        missing = sorted(registered_set - documented_set)
        extra = sorted(documented_set - registered_set)
        raise AssertionError(
            "commands.md differs from generate_handler!: "
            f"missing={missing or 'none'}, extra={extra or 'none'}"
        )
    if len(registered) != len(documented):
        raise AssertionError(
            f"command count differs: registered={len(registered)}, "
            f"documented={len(documented)}"
        )

    expected = len(registered)
    for source in COUNT_DOCS:
        relative = source.relative_to(ROOT)
        mentions = command_count_mentions((root / relative).read_text())
        if not mentions:
            raise AssertionError(f"{relative}: missing command-count statement")
        stale = [count for count in mentions if count != expected]
        if stale:
            raise AssertionError(
                f"{relative}: stale command count(s) {stale}; expected {expected}"
            )
    return expected


def main() -> None:
    count = validate_reference_docs()
    print(
        f"reference docs valid: {count} registered Tauri commands, "
        f"{count} documented command rows"
    )


if __name__ == "__main__":
    main()
