#!/usr/bin/env python3
"""Verify that Tauri command reference docs match the registered command API."""

from __future__ import annotations

from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
LIB_RS = ROOT / "app/src-tauri/src/lib.rs"
SHADOW_COMMANDS_RS = ROOT / "app/src-tauri/src/commands/recording/shadow_commands.rs"
COMMANDS_DOC = ROOT / "docs/reference/commands.md"
COUNT_DOCS = (
    ROOT / "CLAUDE.md",
    COMMANDS_DOC,
    ROOT / "docs/ARCHITECTURE.md",
    ROOT / "docs/FEATURES.md",
)


SHADOW_GATE = 'feature = "dictation-shadow"'
ENTRY = re.compile(r"^\s*(?:[A-Za-z_][A-Za-z0-9_]*::)*([A-Za-z_][A-Za-z0-9_]*)\s*,?\s*$")
CFG_LINE = re.compile(r"^\s*#\[cfg\((.*)\)\]\s*$")
RENAMED_WRAPPER = re.compile(
    r'#\[tauri::command\(rename = "([A-Za-z_][A-Za-z0-9_]*)"\)\]\s*'
    r"(?:pub(?:\([a-z:]+\))?\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)"
)


def renamed_commands(shadow_commands: str) -> dict[str, str]:
    """Rust name -> IPC name of every feature-only wrapper command."""
    return {fn: ipc for ipc, fn in RENAMED_WRAPPER.findall(shadow_commands)}


def registered_commands(lib_rs: str, shadow_commands: str | None = None) -> list[str]:
    # The handler may be wrapped by the main-thread diagnostics instrument,
    # which times each command without changing the registered set.
    handler = re.search(
        r"\.invoke_handler\("
        r"(?:main_thread_watch::instrument_invoke_handler\()?"
        r"tauri::generate_handler!\[(?P<body>.*?)\]\)",
        lib_rs,
        re.S,
    )
    if handler is None:
        raise AssertionError("lib.rs: missing Tauri generate_handler! registration")
    unrecognized = "lib.rs: generate_handler! contains an unrecognized command entry"
    commands: list[str] = []
    # A shadow-gated pair: the original (`cfg(not(feature))`) and its feature-only
    # witnessed wrapper (`cfg(feature)`), registered under one IPC name.
    pending_gate: str | None = None
    original: str | None = None
    wrapper: str | None = None
    for line in handler.group("body").splitlines():
        if not line.strip() or line.lstrip().startswith("//"):
            continue
        cfg = CFG_LINE.match(line)
        if cfg is not None:
            gate = cfg.group(1).strip()
            if pending_gate is not None:
                raise AssertionError(unrecognized)
            if gate == SHADOW_GATE:
                pending_gate = "feature"
            elif gate == f"not({SHADOW_GATE})":
                pending_gate = "not"
            else:
                raise AssertionError(unrecognized)
            continue
        entry = ENTRY.match(line)
        if entry is None:
            raise AssertionError(unrecognized)
        name = entry.group(1)
        if pending_gate is None:
            if original is not None or wrapper is not None:
                raise AssertionError(unrecognized)
            commands.append(name)
            continue
        if pending_gate == "not":
            if original is not None:
                raise AssertionError(unrecognized)
            original = name
        else:
            if wrapper is not None:
                raise AssertionError(unrecognized)
            wrapper = name
        pending_gate = None
        if original is not None and wrapper is not None:
            if shadow_commands is None:
                shadow_commands = SHADOW_COMMANDS_RS.read_text()
            ipc = renamed_commands(shadow_commands).get(wrapper)
            if ipc != original:
                raise AssertionError(
                    f"lib.rs: shadow wrapper `{wrapper}` is registered as {ipc!r}, "
                    f"but its gated pair is `{original}`"
                )
            commands.append(ipc)
            original = wrapper = None
    if pending_gate is not None or original is not None or wrapper is not None:
        raise AssertionError(unrecognized)
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
        (root / LIB_RS.relative_to(ROOT)).read_text(),
        (root / SHADOW_COMMANDS_RS.relative_to(ROOT)).read_text(),
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
