#!/usr/bin/env python3
"""Verify that the native request reference matches the in-process service facade."""

from __future__ import annotations

from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
LIB_RS = ROOT / "app/crates/murmur-services/src/native_services/commands.rs"
COMMANDS_DOC = ROOT / "docs/reference/commands.md"
COUNT_DOCS = (
    ROOT / "CLAUDE.md",
    COMMANDS_DOC,
    ROOT / "docs/ARCHITECTURE.md",
    ROOT / "docs/FEATURES.md",
)


def registered_commands(source: str) -> list[str]:
    commands = re.findall(r'"([a-z_0-9]+)"\s*=>', source)
    if not commands:
        raise AssertionError("native facade: missing request dispatch")
    if len(commands) != len(set(commands)):
        raise AssertionError("native facade: duplicate request names")
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
            "commands.md differs from native request facade: "
            f"missing={missing or 'none'}, extra={extra or 'none'}"
        )
    if len(registered) != len(documented):
        raise AssertionError(
            f"command count differs: registered={len(registered)}, "
            f"documented={len(documented)}"
        )

    expected = len(registered)
    return expected


def main() -> None:
    count = validate_reference_docs()
    print(
        f"reference docs valid: {count} native requests, "
        f"{count} documented command rows"
    )


if __name__ == "__main__":
    main()
