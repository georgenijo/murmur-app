#!/usr/bin/env python3
"""murmur-diag: MCP server for querying Murmur's structured telemetry."""

import json
import os
import re
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any

from mcp.server.fastmcp import FastMCP

mcp = FastMCP("murmur-diag")

DEFAULT_LOG_DIR = Path.home() / "Library" / "Application Support" / "local-dictation" / "logs"
LOG_DIR = Path(os.environ.get("MURMUR_LOG_DIR", DEFAULT_LOG_DIR)).expanduser()

STRUCTURED_LOG_PATTERNS = (
    "events.jsonl*",
    "events.dev.jsonl*",
)
PRETTY_LOG_PATTERNS = (
    "app.log*",
    "app.dev.log*",
)

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def parse_relative_time(s: str) -> datetime:
    """Convert relative time string like '1h', '24h', '7d' to an absolute UTC datetime."""
    m = re.fullmatch(r"(\d+)([smhd])", s.strip())
    if not m:
        raise ValueError(f"Invalid relative time: {s!r}. Use e.g. '1h', '7d'.")
    n, unit = int(m.group(1)), m.group(2)
    delta = {"s": timedelta(seconds=n), "m": timedelta(minutes=n),
             "h": timedelta(hours=n), "d": timedelta(days=n)}[unit]
    return datetime.now(timezone.utc) - delta


def parse_time(s: str | None) -> datetime | None:
    """Parse an ISO timestamp or relative time string. Returns None if input is None."""
    if s is None:
        return None
    s = s.strip()
    if re.fullmatch(r"\d+[smhd]", s):
        return parse_relative_time(s)
    # ISO 8601 — handle trailing Z
    s = s.replace("Z", "+00:00")
    dt = datetime.fromisoformat(s)
    if dt.tzinfo is None:
        dt = dt.replace(tzinfo=timezone.utc)
    return dt


def parse_event_ts(ts_str: str) -> datetime:
    """Parse an event timestamp string to datetime."""
    return datetime.fromisoformat(ts_str.replace("Z", "+00:00"))


def log_build(path: Path) -> str:
    """Return the Murmur build identity represented by a log filename."""
    return "dev" if ".dev." in path.name else "release"


def select_log_files(*patterns: str) -> list[Path]:
    """Resolve log globs once, deduplicating overlapping paths and aliases."""
    selected: list[Path] = []
    seen: set[tuple[int, int] | str] = set()

    for pattern in patterns:
        for path in sorted(LOG_DIR.glob(pattern)):
            if not path.is_file():
                continue
            try:
                stat = path.stat()
                identity: tuple[int, int] | str = (stat.st_dev, stat.st_ino)
            except OSError:
                identity = str(path.resolve())
            if identity in seen:
                continue
            seen.add(identity)
            selected.append(path)

    return selected


def read_jsonl_files(*patterns: str) -> list[dict[str, Any]]:
    """Read release and dev JSONL files once, sorted by timestamp."""
    if not patterns:
        patterns = STRUCTURED_LOG_PATTERNS
    events: list[dict[str, Any]] = []
    for path in select_log_files(*patterns):
        source = {"build": log_build(path), "file": path.name}
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            for line in f:
                line = line.strip()
                if not line:
                    continue
                try:
                    event = json.loads(line)
                    if isinstance(event, dict):
                        event["diag_source"] = source
                        events.append(event)
                except json.JSONDecodeError:
                    continue
    events.sort(key=lambda e: e.get("timestamp", ""))
    return events


def event_build(event: dict[str, Any]) -> str:
    """Return source build metadata attached during JSONL ingestion."""
    source = event.get("diag_source", {})
    return source.get("build", "unknown") if isinstance(source, dict) else "unknown"


def parse_pretty_log_line(raw: str) -> tuple[str | None, str | None, str]:
    """Parse current tracing output and the legacy bracketed log format."""
    timestamp = r"(\d{4}-\d{2}-\d{2}T[\d:.]+(?:Z|[+-]\d{2}:\d{2})?)"
    tracing_match = re.match(
        rf"^{timestamp}\s+(TRACE|DEBUG|INFO|WARN|ERROR)\s+\S+:\s?(.*)$",
        raw,
    )
    if tracing_match:
        return tracing_match.group(1), tracing_match.group(2), tracing_match.group(3)

    legacy_match = re.match(rf"^{timestamp}\s+\[(\w+)]\s+(.*)$", raw)
    if legacy_match:
        return legacy_match.group(1), legacy_match.group(2), legacy_match.group(3)

    return None, None, raw


def pair_keyboard_to_recordings(
    kb_starts: list[dict],
    rec_starts: list[dict],
    all_events: list[dict],
    max_gap_ms: int = 500,
) -> tuple[int, list[dict]]:
    """Match keyboard emit events to recording starts within a time gap.

    Returns (correlated_count, missed_list) where missed_list contains keyboard
    events that had no corresponding recording start within max_gap_ms.
    """
    max_gap = timedelta(milliseconds=max_gap_ms)
    correlated = 0
    missed = []
    rec_idx = 0

    for kb in kb_starts:
        try:
            kb_ts = parse_event_ts(kb["timestamp"])
        except (ValueError, KeyError):
            continue

        found = False
        while rec_idx < len(rec_starts):
            try:
                rec_ts = parse_event_ts(rec_starts[rec_idx]["timestamp"])
            except (ValueError, KeyError):
                rec_idx += 1
                continue
            if rec_ts < kb_ts:
                rec_idx += 1
                continue
            gap = rec_ts - kb_ts
            if gap <= max_gap:
                correlated += 1
                found = True
                rec_idx += 1
            break

        if not found:
            next_ev = None
            gap_ms_val = None
            for e in all_events:
                try:
                    e_ts = parse_event_ts(e["timestamp"])
                except (ValueError, KeyError):
                    continue
                if e_ts > kb_ts:
                    next_ev = e.get("summary", "")
                    gap_ms_val = int((e_ts - kb_ts).total_seconds() * 1000)
                    break
            missed.append({
                "timestamp": kb.get("timestamp"),
                "summary": kb.get("summary"),
                "next_event": next_ev,
                "gap_ms": gap_ms_val,
            })

    return correlated, missed


def extract_kb_starts(events: list[dict]) -> list[dict]:
    """Return only keyboard events that start a recording.

    Walks events in timestamp order and maintains a synthetic `is_recording`
    state that mirrors DoubleTapDetector.recording in keyboard.rs, so
    double-tap-toggle events are classified as start vs stop based on context.
    Hold-down-stop events are excluded.

    Matched summaries (see app/src-tauri/src/keyboard.rs lines 608/623/627/635):
      - "hold-down-start"    → "BOTH -> timer promoted to hold-down-start"
      - "hold-down-stop"     → "BOTH -> emit hold-down-stop (promoted hold)"
      - "double-tap-toggle"  → "BOTH -> emit double-tap-toggle"
                               "BOTH -> emit double-tap-toggle (hold=None)"

    TODO: when issue #152 (correlation tokens) lands and adds data.direction,
    this stateful classifier can be replaced by a simple data.direction == "start" filter.
    """
    sorted_events = sorted(events, key=lambda e: e.get("timestamp", ""))
    is_recording = False
    starts = []

    for e in sorted_events:
        if e.get("stream") != "keyboard":
            continue
        summary = e.get("summary", "")
        if "hold-down-start" in summary:
            is_recording = True
            starts.append(e)
        elif "hold-down-stop" in summary:
            is_recording = False
        elif "double-tap-toggle" in summary:
            if not is_recording:
                starts.append(e)
                is_recording = True
            else:
                is_recording = False

    return starts


def filter_events(
    events: list[dict],
    stream: list[str] | None = None,
    level: list[str] | None = None,
    since: datetime | None = None,
    until: datetime | None = None,
    pattern: str | None = None,
) -> list[dict]:
    """Filter events by stream, level, time range, and summary pattern."""
    pat = re.compile(pattern, re.IGNORECASE) if pattern else None
    result = []
    for ev in events:
        if stream and ev.get("stream") not in stream:
            continue
        if level and ev.get("level") not in level:
            continue
        ts_str = ev.get("timestamp")
        if ts_str:
            try:
                ts = parse_event_ts(ts_str)
            except (ValueError, TypeError):
                continue
            if since and ts < since:
                continue
            if until and ts > until:
                continue
        if pat and not pat.search(ev.get("summary", "")):
            continue
        result.append(ev)
    return result


# ---------------------------------------------------------------------------
# Tools
# ---------------------------------------------------------------------------

@mcp.tool()
def query_events(
    stream: list[str] | None = None,
    level: list[str] | None = None,
    since: str | None = None,
    until: str | None = None,
    pattern: str | None = None,
    limit: int = 50,
    offset: int = 0,
) -> dict:
    """Filter and search structured events from release and dev JSONL logs.

    Args:
        stream: Filter by stream name (keyboard, pipeline, audio, system).
        level: Filter by level (info, warn, error).
        since: Start time — ISO timestamp or relative ('1h', '24h', '7d').
        until: End time — ISO timestamp or relative.
        pattern: Regex matched against the summary field.
        limit: Max results to return (default 50).
        offset: Skip this many results for pagination.
    """
    since_dt = parse_time(since)
    until_dt = parse_time(until)
    all_events = read_jsonl_files()
    matched = filter_events(all_events, stream=stream, level=level,
                            since=since_dt, until=until_dt, pattern=pattern)
    total = len(matched)
    page = matched[offset:offset + limit]
    time_range = {}
    if matched:
        time_range = {"first": matched[0].get("timestamp"), "last": matched[-1].get("timestamp")}
    return {
        "events": page,
        "total_matched": total,
        "time_range": time_range,
        "sources": sorted({event_build(event) for event in matched}),
    }


@mcp.tool()
def correlate_keyboard(
    since: str | None = "24h",
    until: str | None = None,
    max_gap_ms: int = 500,
) -> dict:
    """Correlate keyboard hotkey events with same-build recording starts.

    Answers: 'did every hotkey press result in a recording?'

    Args:
        since: Start time (default '24h').
        until: End time.
        max_gap_ms: Max milliseconds between keyboard emit and recording start to count as correlated (default 500).
    """
    since_dt = parse_time(since)
    until_dt = parse_time(until)
    all_events = read_jsonl_files()
    filtered = filter_events(all_events, since=since_dt, until=until_dt)

    total_keyboard_starts = 0
    total_recording_starts = 0
    total_correlated = 0
    missed: list[dict] = []
    overlap_entries: list[dict] = []
    source_results: dict[str, dict[str, int]] = {}

    # Never correlate two concurrently-running build identities to each other.
    for source in sorted({event_build(event) for event in filtered}):
        source_events = [event for event in filtered if event_build(event) == source]
        kb_starts = extract_kb_starts(source_events)
        rec_starts = [
            event for event in source_events
            if event.get("stream") == "pipeline"
            and "start_native_recording" in event.get("summary", "")
            and "already recording" not in event.get("summary", "").lower()
        ]
        overlaps = [
            event for event in source_events
            if "already recording" in event.get("summary", "").lower()
        ]
        correlated, source_missed = pair_keyboard_to_recordings(
            kb_starts, rec_starts, source_events, max_gap_ms=max_gap_ms)
        for entry in source_missed:
            entry["source"] = source
        missed.extend(source_missed)
        overlap_entries.extend({
            "timestamp": event.get("timestamp"),
            "summary": event.get("summary"),
            "source": source,
        } for event in overlaps)
        total_keyboard_starts += len(kb_starts)
        total_recording_starts += len(rec_starts)
        total_correlated += correlated
        source_results[source] = {
            "keyboard_starts": len(kb_starts),
            "recording_starts": len(rec_starts),
            "correlated": correlated,
            "missed": len(source_missed),
            "overlap": len(overlaps),
        }

    return {
        "total_keyboard_starts": total_keyboard_starts,
        "total_recording_starts": total_recording_starts,
        "correlated": total_correlated,
        "missed": missed,
        "overlap": overlap_entries,
        "sources": source_results,
    }


@mcp.tool()
def session_summary(
    since: str | None = "7d",
    limit: int = 10,
) -> dict:
    """High-level view of release and dev app sessions.

    Identifies sessions by 'app setup' events and summarizes each one.

    Args:
        since: Time range start (default '7d').
        limit: Max sessions to return (default 10).
    """
    since_dt = parse_time(since)
    all_events = read_jsonl_files()
    if since_dt:
        all_events = [e for e in all_events
                      if parse_event_ts(e.get("timestamp", "1970-01-01T00:00:00Z")) >= since_dt]

    sessions = []
    for source in sorted({event_build(event) for event in all_events}):
        source_events = [event for event in all_events if event_build(event) == source]
        session_starts = [
            i for i, event in enumerate(source_events)
            if event.get("summary", "").startswith("app setup")
        ]

        for j, start_idx in enumerate(session_starts):
            end_idx = (
                session_starts[j + 1]
                if j + 1 < len(session_starts)
                else len(source_events)
            )
            session_events = source_events[start_idx:end_idx]
            if not session_events:
                continue

            setup_ev = session_events[0]
            version_match = re.search(r"v([\d.]+)", setup_ev.get("summary", ""))
            version = version_match.group(0) if version_match else "unknown"

            recordings = sum(1 for e in session_events
                             if "start_native_recording" in e.get("summary", "")
                             and "already recording" not in e.get("summary", "").lower())
            kb_events = sum(1 for e in session_events if e.get("stream") == "keyboard")
            errors = sum(1 for e in session_events if e.get("level") == "error")
            warnings = sum(1 for e in session_events if e.get("level") == "warn")

            # Missed hotkeys using the same pairing logic as correlate_keyboard
            kb_starts = extract_kb_starts(session_events)
            rec_starts = [e for e in session_events if e.get("stream") == "pipeline"
                          and "start_native_recording" in e.get("summary", "")
                          and "already recording" not in e.get("summary", "").lower()]
            _, missed_list = pair_keyboard_to_recordings(kb_starts, rec_starts, session_events)
            missed_hotkeys = len(missed_list)

            # Peak RSS from heartbeat/baseline data
            peak_rss: float | None = None
            for e in session_events:
                rss = e.get("data", {}).get("rss_mb")
                if rss is not None:
                    if peak_rss is None or rss > peak_rss:
                        peak_rss = rss

            sessions.append({
                "started": session_events[0].get("timestamp"),
                "ended": session_events[-1].get("timestamp"),
                "version": version,
                "source": source,
                "recordings": recordings,
                "keyboard_events": kb_events,
                "errors": errors,
                "warnings": warnings,
                "missed_hotkeys": missed_hotkeys,
                "peak_rss_mb": peak_rss,
            })

    # Return most recent sessions first
    sessions.sort(key=lambda session: session.get("started", ""), reverse=True)
    return {"sessions": sessions[:limit], "total_sessions": len(sessions)}


@mcp.tool()
def check_health() -> dict:
    """Quick diagnostic snapshot — is the app working right now?

    Returns the most recent keyboard event, recording, error, listener status,
    session uptime, and whether processing appears stuck.
    """
    all_events = read_jsonl_files()
    now = datetime.now(timezone.utc)

    def find_last(predicate):
        for ev in reversed(all_events):
            if predicate(ev):
                return ev
        return None

    def age_entry(ev):
        if ev is None:
            return None
        ts_str = ev.get("timestamp")
        if not ts_str:
            return None
        try:
            ts = parse_event_ts(ts_str)
            age = (now - ts).total_seconds()
        except (ValueError, TypeError):
            return None
        return {
            "timestamp": ts_str,
            "summary": ev.get("summary"),
            "age_seconds": round(age),
            "source": event_build(ev),
        }

    last_kb = find_last(lambda e: e.get("stream") == "keyboard")
    last_rec = find_last(lambda e: "start_native_recording" in e.get("summary", "")
                         and "already recording" not in e.get("summary", "").lower())
    last_err = find_last(lambda e: e.get("level") in ("error", "warn"))

    # Listener active: last "Keyboard listener started" without a subsequent stop
    listener_active = False
    for ev in reversed(all_events):
        s = ev.get("summary", "")
        if "Keyboard listener started" in s:
            listener_active = True
            break
        if "Keyboard listener stopped" in s:
            listener_active = False
            break

    # Session uptime: time since last "app setup"
    session_uptime_minutes: float | None = None
    last_setup = find_last(lambda e: e.get("summary", "").startswith("app setup"))
    if last_setup:
        try:
            setup_ts = parse_event_ts(last_setup["timestamp"])
            session_uptime_minutes = round((now - setup_ts).total_seconds() / 60, 1)
        except (ValueError, KeyError):
            pass

    # Processing stuck: last status was "processing" with no idle follow-up for >30s
    is_stuck = False
    last_status = find_last(lambda e: "recording-status-changed" in e.get("summary", "")
                            or "status" in e.get("summary", "").lower())
    if last_status:
        summary = last_status.get("summary", "")
        if "processing" in summary.lower():
            try:
                status_ts = parse_event_ts(last_status["timestamp"])
                if (now - status_ts).total_seconds() > 30:
                    is_stuck = True
            except (ValueError, KeyError):
                pass

    return {
        "last_keyboard_event": age_entry(last_kb),
        "last_recording": age_entry(last_rec),
        "last_error": age_entry(last_err),
        "listener_active": listener_active,
        "current_session_uptime_minutes": session_uptime_minutes,
        "is_processing_likely_stuck": is_stuck,
    }


@mcp.tool()
def search_logs(
    pattern: str,
    since: str | None = None,
    until: str | None = None,
    context: int = 0,
    limit: int = 50,
) -> dict:
    """Search release and dev text logs for detail absent from structured events.

    Args:
        pattern: Regex to match against log lines.
        since: Start time — ISO timestamp or relative ('1h', '7d').
        until: End time — ISO timestamp or relative.
        context: Lines of context around each match (default 0).
        limit: Max results (default 50).
    """
    since_dt = parse_time(since)
    until_dt = parse_time(until)
    pat = re.compile(pattern, re.IGNORECASE)

    log_files = select_log_files(*PRETTY_LOG_PATTERNS)

    # Read all lines with parsed metadata
    all_lines: list[dict] = []
    for path in log_files:
        if not path.exists():
            continue
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            for line_num, raw in enumerate(f, 1):
                raw = raw.rstrip("\n")
                ts_str, level, message = parse_pretty_log_line(raw)
                all_lines.append({
                    "line_number": line_num,
                    "file": path.name,
                    "source": log_build(path),
                    "timestamp": ts_str,
                    "level": level,
                    "message": message,
                    "raw": raw,
                })

    # Filter by time range
    if since_dt or until_dt:
        filtered = []
        for entry in all_lines:
            ts_str = entry.get("timestamp")
            if not ts_str:
                filtered.append(entry)  # keep lines without timestamps (continuation lines)
                continue
            try:
                ts = parse_event_ts(ts_str)
            except (ValueError, TypeError):
                filtered.append(entry)
                continue
            if since_dt and ts < since_dt:
                continue
            if until_dt and ts > until_dt:
                continue
            filtered.append(entry)
        all_lines = filtered

    # Search
    matches = []
    total_matched = 0
    for i, entry in enumerate(all_lines):
        if pat.search(entry["raw"]):
            total_matched += 1
            if len(matches) < limit:
                ctx_before = [all_lines[j]["raw"] for j in range(max(0, i - context), i)] if context else []
                ctx_after = [all_lines[j]["raw"]
                             for j in range(i + 1, min(len(all_lines), i + 1 + context))] if context else []
                matches.append({
                    "line_number": entry["line_number"],
                    "file": entry["file"],
                    "source": entry["source"],
                    "timestamp": entry["timestamp"],
                    "level": entry["level"],
                    "message": entry["message"],
                    "context_before": ctx_before,
                    "context_after": ctx_after,
                })

    return {
        "matches": matches,
        "total_matched": total_matched,
        "sources": sorted({entry["source"] for entry in all_lines}),
    }


if __name__ == "__main__":
    mcp.run(transport="stdio")
