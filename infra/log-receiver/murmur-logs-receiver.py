#!/usr/bin/env python3
"""Murmur log ingest receiver + fleet dashboard.

Accepts NDJSON batches from murmur-app installs and appends them to
per-install JSONL files under ~/murmur-logs/. Stdlib only.

    POST /ingest
      Authorization: Bearer <token>
      X-Install-Id: <uuid4>
      X-App-Version: <semver>   (optional)
      X-Dev: 1                  (optional, dev builds)
      body: NDJSON (one JSON object per line)

    GET /dashboard   — HTML overview of every install (gate behind CF Access)
    GET /healthz     — liveness

Responses: 204 ok, 401 bad token, 400 bad payload, 413 too large.
"""

import html
import json
import os
import re
import sys
import time
from datetime import datetime
from zoneinfo import ZoneInfo
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

TOKEN = "a1b4068693a1f3868bcf03c01ebcf1e9f000080b3e8bfcb0"
ROOT = os.path.expanduser("~/murmur-logs")
MAX_BODY = 8 * 1024 * 1024  # 8 MB
MAX_FILE = 200 * 1024 * 1024  # per-install cap: stop appending past 200 MB
INSTALL_ID_RE = re.compile(r"^[0-9a-fA-F-]{8,64}$")
MAX_INSTALLS = 150
MAX_TOTAL = 10 * 1024 * 1024 * 1024  # 10 GB across all installs
_usage_cache = {"t": 0.0, "bytes": 0, "dirs": 0}


def root_usage():
    """Total bytes and dir count under ROOT, cached for 60s."""
    if time.time() - _usage_cache["t"] > 60:
        total = dirs = 0
        for entry in os.listdir(ROOT) if os.path.isdir(ROOT) else []:
            d = os.path.join(ROOT, entry)
            if not os.path.isdir(d):
                continue
            dirs += 1
            for f in os.listdir(d):
                try:
                    total += os.path.getsize(os.path.join(d, f))
                except OSError:
                    pass
        _usage_cache.update(t=time.time(), bytes=total, dirs=dirs)
    return _usage_cache


def atomic_write_json(path, obj):
    tmp = path + ".tmp"
    try:
        with open(tmp, "w") as f:
            json.dump(obj, f)
        os.replace(tmp, path)
    except OSError:
        pass
EASTERN = ZoneInfo("America/New_York")


def eastern_time(ts):
    """'2026-07-30T13:42:15.192Z' -> '09:42:15' Eastern."""
    try:
        dt = datetime.fromisoformat(ts.replace("Z", "+00:00"))
        return dt.astimezone(EASTERN).strftime("%H:%M:%S")
    except (ValueError, TypeError):
        return str(ts)[11:19]


def tail_lines(path, n, max_bytes=64 * 1024):
    """Last n lines of a file, reading at most max_bytes from the end."""
    try:
        size = os.path.getsize(path)
        with open(path, "rb") as f:
            f.seek(max(0, size - max_bytes))
            chunk = f.read().decode("utf-8", "replace")
        lines = [l for l in chunk.split("\n") if l.strip()]
        return lines[-n:]
    except OSError:
        return []


def count_lines(path):
    try:
        with open(path, "rb") as f:
            return sum(buf.count(b"\n") for buf in iter(lambda: f.read(1 << 20), b""))
    except OSError:
        return 0


def collect_installs():
    installs = []
    if not os.path.isdir(ROOT):
        return installs
    for entry in sorted(os.listdir(ROOT)):
        d = os.path.join(ROOT, entry)
        if not os.path.isdir(d):
            continue
        meta = {}
        try:
            with open(os.path.join(d, "meta.json")) as f:
                meta = json.load(f)
        except (OSError, ValueError):
            pass
        state = {}
        try:
            with open(os.path.join(d, "state.json")) as f:
                state = json.load(f)
        except (OSError, ValueError):
            pass
        for fname in ("events.jsonl", "events.dev.jsonl"):
            path = os.path.join(d, fname)
            if not os.path.exists(path):
                continue
            last = tail_lines(path, 1)
            last_event, last_ts = {}, ""
            if last:
                try:
                    last_event = json.loads(last[-1])
                    last_ts = last_event.get("timestamp", "")
                except ValueError:
                    pass
            installs.append({
                "id": entry,
                "kind": "dev" if ".dev." in fname else "prod",
                "device": meta.get("device_name", ""),
                "os": meta.get("os", ""),
                "hw": meta.get("hw", ""),
                "specs": meta.get("specs", ""),
                "mic": state.get("default_input") or "",
                "version": meta.get("last_version", "?"),
                "events": count_lines(path),
                "bytes": os.path.getsize(path),
                "mtime": os.path.getmtime(path),
                "first_seen": os.path.getctime(d),
                "last_ts": last_ts,
                "last_summary": str(last_event.get("summary", ""))[:110],
            })
    installs.sort(key=lambda i: i["mtime"], reverse=True)
    return installs


def ago(ts):
    s = int(time.time() - ts)
    if s < 90:
        return "%ds ago" % s
    if s < 5400:
        return "%dm ago" % (s // 60)
    if s < 172800:
        return "%dh ago" % (s // 3600)
    return "%dd ago" % (s // 86400)


HEALTH_AREAS = (
    ("microphone", "Microphone"),
    ("shortcuts", "Shortcuts"),
    ("dictation", "Dictation"),
    ("updates", "Updates"),
    ("transforms", "Transforms"),
)

EVENT_CODE_COMPATIBILITY = (
    ("rdev listener thread started", "keyboard.listener_started"),
    ("listener heartbeat — no rdev callbacks observed", "keyboard.listener_silent"),
    ("rdev listener error:", "keyboard.listener_failed"),
    (
        "capture backend exceeded its active initialization budget",
        "audio.capture_backend_timeout",
    ),
    (
        "capture backend failed before retained audio; trying bounded fallback",
        "audio.fallback_started",
    ),
    ("audio readiness accepted", "audio.capture_ready"),
    (
        "both capture backend attempts failed before first PCM",
        "audio.capture_failed",
    ),
    ("audio lifecycle failed", "audio.lifecycle_failed"),
    ("transcription complete", "pipeline.dictation_completed"),
    ("stop_native_recording: pipeline failed:", "pipeline.dictation_failed"),
    ("transform_pass_outcome", "transform.pass_outcome"),
    ("[updater] no update available", "updater.check_current"),
    ("[updater] check failed", "updater.check_failed"),
    (
        "[updater] install blocked by macOS App Translocation",
        "updater.install_blocked",
    ),
    ("[updater] installed, relaunching", "updater.install_ready"),
    ("[updater] download/install failed", "updater.install_failed"),
)

STATUS_PRIORITY = {
    "healthy": 0,
    "diagnostic": 1,
    "recovered": 2,
    "degraded": 3,
    "action": 4,
}

STATUS_LABELS = {
    "healthy": "OK",
    "diagnostic": "FYI",
    "recovered": "Recovered",
    "degraded": "Watch",
    "action": "Action",
}


def event_code(event):
    """Return a stable event code, with a bounded fallback for old JSONL."""
    data = event.get("data")
    if isinstance(data, dict):
        code = data.get("event_code")
        if isinstance(code, str) and re.match(r"^[a-z][a-z0-9_.-]{2,80}$", code):
            return code
    summary = str(event.get("summary", ""))
    for prefix, code in EVENT_CODE_COMPATIBILITY:
        if summary.startswith(prefix):
            return code
    return None


def event_epoch(event):
    try:
        return datetime.fromisoformat(
            str(event.get("timestamp", "")).replace("Z", "+00:00")
        ).timestamp()
    except (ValueError, TypeError):
        return 0.0


def display_event_time(event):
    try:
        dt = datetime.fromisoformat(
            str(event.get("timestamp", "")).replace("Z", "+00:00")
        )
        return dt.astimezone(EASTERN).strftime("%b %-d, %H:%M:%S")
    except (ValueError, TypeError):
        return str(event.get("timestamp", ""))[:24] or "unknown"


def event_value(event, key, default=None):
    data = event.get("data")
    return data.get(key, default) if isinstance(data, dict) else default


def event_label(event, key, default):
    """Return a normalized, bounded label for one structured data value."""
    value = event_value(event, key, default)
    text = str(value if value not in (None, "") else default)
    text = re.sub(r"\s+", " ", text).strip()
    return text[:40] or str(default)


def format_duration_ms(value):
    try:
        milliseconds = max(0, int(value))
    except (TypeError, ValueError):
        return None
    if milliseconds < 1_000:
        return "%d ms" % milliseconds
    if milliseconds < 60_000:
        return "%.1f seconds" % (milliseconds / 1_000)
    if milliseconds < 3_600_000:
        return "%d minutes" % (milliseconds // 60_000)
    hours = milliseconds / 3_600_000
    return "%.1f hours" % hours


def signal(
    area,
    status,
    code,
    title,
    explanation,
    action,
    events,
    group=None,
):
    source_events = list(events)
    return {
        "area": area,
        "status": status,
        "code": code,
        "group": group or code,
        "title": title,
        "explanation": explanation,
        "action": action,
        "events": source_events,
        "first_epoch": min((event_epoch(e) for e in source_events), default=0),
        "last_epoch": max((event_epoch(e) for e in source_events), default=0),
        "count": 1,
    }


def classify_event(event):
    """Translate one event without guessing beyond its stable evidence."""
    code = event_code(event)
    if code == "keyboard.listener_started":
        return signal(
            "shortcuts",
            "healthy",
            code,
            "Shortcut listener started",
            "Murmur is listening for the configured global shortcut.",
            "No action required.",
            [event],
        )
    if code == "keyboard.listener_silent":
        elapsed = format_duration_ms(event_value(event, "silent_for_ms"))
        detail = (
            "No global keyboard callback was observed for %s." % elapsed
            if elapsed
            else "No global keyboard callback was observed for a while."
        )
        return signal(
            "shortcuts",
            "diagnostic",
            code,
            "No recent shortcut activity",
            detail + " This is usually normal while the Mac is idle or asleep.",
            "Check only if Murmur's shortcut has stopped responding.",
            [event],
        )
    if code == "keyboard.listener_failed":
        return signal(
            "shortcuts",
            "action",
            code,
            "Shortcut listener failed",
            "Murmur's global keyboard listener exited with an error.",
            "Restart Murmur, then verify Accessibility permission if shortcuts still fail.",
            [event],
        )
    if code == "audio.capture_backend_timeout":
        backend = event_label(event, "backend", "primary")
        setup_step = event_label(event, "last_setup_step", "none")
        where = (
            " while opening %s" % setup_step.replace("_", " ")
            if setup_step != "none"
            else ""
        )
        return signal(
            "microphone",
            "degraded",
            code,
            "A microphone backend timed out",
            "The %s capture backend took too long%s." % (backend, where),
            "Murmur may recover by switching backends; check the later outcome.",
            [event],
        )
    if code == "audio.fallback_started":
        source = event_label(event, "from_backend", "primary")
        target = event_label(event, "to_backend", "backup")
        return signal(
            "microphone",
            "degraded",
            code,
            "Murmur switched microphone backends",
            "The %s backend failed before audio arrived, so Murmur tried %s."
            % (source, target),
            "The loaded events do not yet prove whether fallback succeeded.",
            [event],
        )
    if code == "audio.capture_ready":
        startup = format_duration_ms(event_value(event, "startup_ms"))
        detail = "Microphone audio became ready"
        if startup:
            detail += " in %s" % startup
        return signal(
            "microphone",
            "healthy",
            code,
            "Microphone started",
            detail + ".",
            "No action required.",
            [event],
        )
    if code in ("audio.capture_failed", "audio.lifecycle_failed"):
        kind = event_label(event, "error_kind", "")
        detail = "Murmur could not start or retain microphone audio."
        if kind:
            detail += " The reported failure was %s." % kind.replace("_", " ")
        return signal(
            "microphone",
            "action",
            code,
            "Microphone failed",
            detail,
            "Try recording again; inspect Technical details if the failure repeats.",
            [event],
            group="audio.capture_failed",
        )
    if code == "pipeline.dictation_completed":
        duration = format_duration_ms(event_value(event, "total_ms"))
        detail = "The most recent recognized dictation completed successfully"
        if duration:
            detail += " in %s" % duration
        return signal(
            "dictation",
            "healthy",
            code,
            "Dictation completed",
            detail + ".",
            "No action required.",
            [event],
        )
    if code == "pipeline.dictation_failed":
        return signal(
            "dictation",
            "action",
            code,
            "Dictation processing failed",
            "Microphone audio reached the processing pipeline, but dictation did not complete.",
            "Retry once; inspect Technical details if it happens again.",
            [event],
        )
    if code == "updater.check_current":
        return signal(
            "updates",
            "healthy",
            code,
            "Murmur is up to date",
            "The latest update check completed and found no newer release.",
            "No action required.",
            [event],
        )
    if code == "updater.check_failed":
        return signal(
            "updates",
            "degraded",
            code,
            "Update check failed",
            "Murmur could not check for a new version.",
            "Murmur will try again later; check the network if this persists.",
            [event],
        )
    if code == "updater.install_blocked":
        return signal(
            "updates",
            "action",
            code,
            "Update installation is blocked",
            "macOS App Translocation prevents Murmur from safely installing the update.",
            "Move Murmur to Applications from Finder, reopen it, and retry.",
            [event],
        )
    if code == "updater.install_ready":
        return signal(
            "updates",
            "healthy",
            code,
            "Update installed",
            "The update finished installing and Murmur began relaunching.",
            "No action required.",
            [event],
        )
    if code == "updater.install_failed":
        return signal(
            "updates",
            "action",
            code,
            "Update installation failed",
            "Murmur could not finish downloading or installing the update.",
            "Retry the update; inspect Technical details if it fails again.",
            [event],
        )
    if code == "transform.pass_outcome":
        outcome = event_label(event, "outcome", "unknown")
        if outcome in ("ok", "ready", "applied", "undone"):
            status = "healthy"
            title = "Transform completed"
            explanation = "The selected-text transform reached %s." % outcome
            action = "No action required."
        elif outcome in ("cancelled", "capture_aborted", "empty"):
            status = "diagnostic"
            title = "Transform did not make a change"
            explanation = "The transform ended as %s." % outcome.replace("_", " ")
            action = "No action required unless this was unexpected."
        elif outcome in ("error", "failed", "transcription_error"):
            status = "action"
            title = "Transform failed"
            explanation = "The selected-text transform did not complete."
            action = "Retry once; inspect Technical details if it repeats."
        else:
            status = "diagnostic"
            title = "Transform outcome recorded"
            explanation = "Murmur recorded a transform outcome it cannot summarize safely."
            action = "Inspect Technical details for the raw outcome."
        group_outcome = (
            outcome
            if outcome
            in (
                "ok",
                "ready",
                "applied",
                "undone",
                "cancelled",
                "capture_aborted",
                "empty",
                "error",
                "failed",
                "transcription_error",
            )
            else "other"
        )
        return signal(
            "transforms",
            status,
            code,
            title,
            explanation,
            action,
            [event],
            group="transform.pass_outcome.%s" % group_outcome,
        )
    return None


def correlation_key(event):
    owner = event_value(event, "owner")
    if isinstance(owner, int) and not isinstance(owner, bool) and 0 <= owner < 2**64:
        return str(owner)
    if isinstance(owner, str) and owner.isdigit() and len(owner) <= 20:
        return str(int(owner))
    return None


def build_health_signals(events):
    """Build ordered health signals, correlating fallback with later readiness."""
    signals = []
    pending_timeouts = {}
    pending_fallbacks = {}
    failure_signals = {}
    for event in events:
        code = event_code(event)
        if code == "audio.capture_backend_timeout":
            key = correlation_key(event)
            if key is None:
                classified = classify_event(event)
                if classified:
                    signals.append(classified)
            else:
                pending_timeouts[key] = event
            continue
        if code == "audio.fallback_started":
            key = correlation_key(event)
            if key is None:
                classified = classify_event(event)
                if classified:
                    signals.append(classified)
                continue
            evidence = []
            timeout = pending_timeouts.pop(key, None)
            if timeout:
                evidence.append(timeout)
            evidence.append(event)
            pending_fallbacks[key] = evidence
            continue
        if code == "audio.capture_ready":
            key = correlation_key(event)
            fallback = pending_fallbacks.pop(key, None) if key is not None else None
            if fallback:
                source = event_label(fallback[-1], "from_backend", "primary")
                target = event_label(fallback[-1], "to_backend", "backup")
                signals.append(
                    signal(
                        "microphone",
                        "recovered",
                        "audio.fallback_recovered",
                        "Microphone recovered with its backup backend",
                        "The %s backend failed, Murmur switched to %s, and audio became ready."
                        % (source, target),
                        "No action required unless fallback happens repeatedly.",
                        fallback + [event],
                    )
                )
                continue
        if code in ("audio.capture_failed", "audio.lifecycle_failed"):
            key = correlation_key(event)
            fallback = pending_fallbacks.pop(key, None) if key is not None else None
            existing_failure = failure_signals.get(key) if key is not None else None
            if existing_failure is not None:
                existing_failure["events"].append(event)
                existing_failure["last_epoch"] = max(
                    existing_failure["last_epoch"], event_epoch(event)
                )
                continue
            classified = classify_event(event)
            if classified:
                if fallback:
                    classified["events"] = fallback + classified["events"]
                    classified["first_epoch"] = min(
                        event_epoch(e) for e in classified["events"]
                    )
                signals.append(classified)
                if key is not None:
                    failure_signals[key] = classified
            continue
        classified = classify_event(event)
        if classified:
            signals.append(classified)

    for event in pending_timeouts.values():
        classified = classify_event(event)
        if classified:
            signals.append(classified)
    for evidence in pending_fallbacks.values():
        classified = classify_event(evidence[-1])
        if classified:
            classified["events"] = evidence
            classified["first_epoch"] = min(event_epoch(e) for e in evidence)
            signals.append(classified)
    signals.sort(key=lambda item: item["last_epoch"])
    return signals


def unknown_problem_signal(event):
    level = str(event.get("level", "info"))
    if level not in ("warn", "error"):
        return None
    summary = str(event.get("summary", ""))[:160] or "Unlabeled technical event"
    status = "action" if level == "error" else "diagnostic"
    return signal(
        "technical",
        status,
        "unknown.%s" % level,
        "Technical error" if level == "error" else "Technical warning",
        summary,
        "Review the raw event; Murmur has no safe plain-English mapping for it yet.",
        [event],
        group="unknown.%s.%s.%s"
        % (level, str(event.get("stream", ""))[:40], summary),
    )


def group_problem_signals(events, signals):
    recognized_ids = {id(event) for item in signals for event in item["events"]}
    problem_signals = [item for item in signals if item["status"] != "healthy"]
    for event in events:
        if id(event) in recognized_ids:
            continue
        unknown = unknown_problem_signal(event)
        if unknown:
            problem_signals.append(unknown)

    grouped = {}
    for item in problem_signals:
        key = item["group"]
        existing = grouped.get(key)
        if existing is None:
            grouped[key] = dict(item)
            grouped[key]["events"] = list(item["events"])
            continue
        existing["count"] += item["count"]
        existing["first_epoch"] = min(existing["first_epoch"], item["first_epoch"])
        existing["last_epoch"] = max(existing["last_epoch"], item["last_epoch"])
        existing["events"].extend(item["events"])
        if STATUS_PRIORITY[item["status"]] > STATUS_PRIORITY[existing["status"]]:
            existing["status"] = item["status"]
            existing["title"] = item["title"]
            existing["explanation"] = item["explanation"]
            existing["action"] = item["action"]

    return sorted(
        grouped.values(),
        key=lambda item: (STATUS_PRIORITY[item["status"]], item["last_epoch"]),
        reverse=True,
    )


def build_health_cards(signals, state):
    cards = {
        area: {
            "area": area,
            "label": label,
            "status": "diagnostic",
            "title": "No recent evidence",
            "explanation": "No recognized health event is present in the loaded log window.",
            "action": "Open the raw timeline for older or unmapped events.",
            "last_epoch": 0,
            "events": [],
        }
        for area, label in HEALTH_AREAS
    }
    for item in signals:
        area = item["area"]
        if area not in cards:
            continue
        cards[area] = {
            "area": area,
            "label": dict(HEALTH_AREAS)[area],
            "status": item["status"],
            "title": item["title"],
            "explanation": item["explanation"],
            "action": item["action"],
            "last_epoch": item["last_epoch"],
            "events": item["events"],
        }
    if isinstance(state, dict) and "default_input_available" in state:
        state_epoch = state.get("received_at", 0)
        if not isinstance(state_epoch, (int, float)):
            state_epoch = 0
        microphone = cards["microphone"]
        if state_epoch >= microphone["last_epoch"]:
            if state.get("input_enumeration_ok") is False:
                microphone.update(
                    status="diagnostic",
                    title="Microphone status unavailable",
                    explanation="Murmur could not enumerate audio inputs in the latest state check.",
                    action="Check again after capture is idle.",
                    last_epoch=state_epoch,
                    events=[],
                )
            elif state.get("default_input_available") is False:
                microphone.update(
                    status="action",
                    title="No default microphone available",
                    explanation="The latest privacy-safe state snapshot found no default input.",
                    action="Connect or select a microphone, then retry.",
                    last_epoch=state_epoch,
                    events=[],
                )
    return [cards[area] for area, _ in HEALTH_AREAS]


def raw_event_row(event):
    level = str(event.get("level", "info"))
    row_class = level if level in ("warn", "error") else ""
    data = event.get("data")
    data = data if isinstance(data, dict) else {}
    data_str = " ".join(
        "%s=%s" % (key, value) for key, value in list(data.items())[:6]
    )
    return (
        '<tr class="%s"><td class="num">%s</td>'
        '<td><span class="stream">%s</span></td>'
        '<td><span class="lvl %s">%s</span></td>'
        '<td><code>%s</code><div class="meta">%s</div></td></tr>'
        % (
            row_class,
            html.escape(eastern_time(str(event.get("timestamp", "")))),
            html.escape(str(event.get("stream", ""))),
            html.escape(level),
            html.escape(level),
            html.escape(str(event.get("summary", ""))[:160]),
            html.escape(data_str[:160]),
        )
    )


def compact_event(event):
    level = str(event.get("level", "info"))
    data = event.get("data")
    data = data if isinstance(data, dict) else {}
    data_str = " ".join(
        "%s=%s" % (key, value) for key, value in list(data.items())[:6]
    )
    return (
        '<div class="health-event %s"><div class="health-event-head">'
        '<span class="num">%s</span><span class="stream">%s</span>'
        '<span class="lvl %s">%s</span></div><code>%s</code>'
        '<div class="meta">%s</div></div>'
        % (
            html.escape(level if level in ("warn", "error") else "info"),
            html.escape(eastern_time(str(event.get("timestamp", "")))),
            html.escape(str(event.get("stream", ""))),
            html.escape(level),
            html.escape(level),
            html.escape(str(event.get("summary", ""))[:160]),
            html.escape(data_str[:160]),
        )
    )


def render_health_card(card):
    seen = (
        "Last evidence %s" % ago(card["last_epoch"])
        if card["last_epoch"]
        else "No recognized event in this window"
    )
    evidence = ""
    if card.get("events"):
        evidence = (
            '<details class="health-evidence"><summary>Technical details</summary>'
            '<div class="health-source">%s</div></details>'
            % "".join(
                compact_event(event)
                for event in sorted(card["events"], key=event_epoch, reverse=True)
            )
        )
    return (
        '<article class="health-card %s">'
        '<div class="health-head"><span class="health-area">%s</span>'
        '<span class="status %s">%s</span></div>'
        '<strong>%s</strong><p>%s</p>'
        '<div class="meta">%s · %s</div>%s</article>'
        % (
            html.escape(card["status"]),
            html.escape(card["label"]),
            html.escape(card["status"]),
            html.escape(STATUS_LABELS.get(card["status"], card["status"])),
            html.escape(card["title"]),
            html.escape(card["explanation"]),
            html.escape(card["action"]),
            html.escape(seen),
            evidence,
        )
    )


def render_problem_group(item):
    evidence = sorted(item["events"], key=event_epoch, reverse=True)
    shown = evidence[:20]
    hidden_note = ""
    if len(evidence) > len(shown):
        hidden_note = (
            '<p class="meta">Showing the newest %d of %d source events. '
            "The raw timeline below retains the complete loaded evidence.</p>"
            % (len(shown), len(evidence))
        )
    first_event = min(evidence, key=event_epoch) if evidence else {}
    last_event = max(evidence, key=event_epoch) if evidence else {}
    occurrence = "1 occurrence" if item["count"] == 1 else "%d occurrences" % item["count"]
    return (
        '<details class="problem-card %s"><summary>'
        '<span><span class="status %s">%s</span><strong>%s</strong>'
        '<span class="problem-copy">%s</span></span>'
        '<span class="problem-count">%s</span></summary>'
        '<div class="problem-detail"><p>%s</p><p><strong>%s</strong></p>'
        '<p class="meta">First seen %s · Last seen %s</p>'
        "%s<table><tbody>%s</tbody></table></div></details>"
        % (
            html.escape(item["status"]),
            html.escape(item["status"]),
            html.escape(STATUS_LABELS.get(item["status"], item["status"])),
            html.escape(item["title"]),
            html.escape(item["explanation"]),
            html.escape(occurrence),
            html.escape(item["explanation"]),
            html.escape(item["action"]),
            html.escape(display_event_time(first_event)),
            html.escape(display_event_time(last_event)),
            hidden_note,
            "".join(raw_event_row(event) for event in shown),
        )
    )


def render_dashboard():
    installs = collect_installs()
    now = time.time()
    rows = []
    for i in installs:
        fresh = now - i["mtime"] < 300
        is_new = now - i["first_seen"] < 86400
        dot = "#4ade80" if fresh else "#64748b"
        badge = ' <span class="new">NEW</span>' if is_new else ""
        kind_cls = "dev" if i["kind"] == "dev" else "prod"
        device = html.escape(i["device"]) if i["device"] else "&mdash;"
        os_hw = " · ".join(x for x in (i["os"], i["specs"] or i["hw"]) if x)
        rows.append(
            '<tr>'
            '<td><span class="dot" style="background:%s"></span>'
            '<a href="/install/%s?kind=%s"><strong>%s</strong></a>'
            ' <span class="badge %s">%s</span>%s'
            '<div class="meta">%s &middot; %s events &middot; %.1f&thinsp;MB%s</div></td>'
            '<td class="num">v%s</td>'
            '<td>%s</td>'
            '</tr>'
            % (
                dot,
                html.escape(i["id"]),
                i["kind"],
                device,
                kind_cls,
                i["kind"],
                badge,
                html.escape(os_hw) or "&mdash;",
                "{:,}".format(i["events"]),
                i["bytes"] / 1048576,
                " &middot; &#127908; " + html.escape(i["mic"]) if i["mic"] else "",
                html.escape(str(i["version"])),
                ago(i["mtime"]),
            )
        )
    body = (
        "<h1>murmur fleet logs</h1>"
        "<p class='sub'>%d install stream%s · refreshes every 30s · %s</p>"
        "<table><thead><tr><th>device</th><th>version</th>"
        "<th>last event</th></tr></thead>"
        "<tbody>%s</tbody></table>"
        % (
            len(installs),
            "" if len(installs) == 1 else "s",
            datetime.now(EASTERN).strftime("%Y-%m-%d %-I:%M %p ET"),
            "".join(rows) or '<tr><td colspan="3">no installs yet</td></tr>',
        )
    )
    return """<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="refresh" content="30">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>murmur fleet logs</title><style>
body{background:#0b1120;color:#e2e8f0;font:15px/1.5 -apple-system,system-ui,sans-serif;margin:2rem auto;max-width:78rem;padding:0 1rem}
h1{font-size:1.3rem;margin:0}
h2{font-size:1rem;margin:1.7rem 0 .7rem}
.sub{color:#64748b;margin:.3rem 0 1.4rem;font-size:.85rem}
table{border-collapse:collapse;width:100%%}
th{text-align:left;color:#64748b;font-size:.75rem;text-transform:uppercase;letter-spacing:.05em;padding:.4rem .7rem;border-bottom:1px solid #1e293b}
td{padding:.55rem .7rem;border-bottom:1px solid #16213a;vertical-align:top}
td.num{text-align:right;font-variant-numeric:tabular-nums}
code{color:#93c5fd;font-size:.85em}
.summary code{color:#94a3b8}
.dot{display:inline-block;width:8px;height:8px;border-radius:50%%;margin-right:.5rem}
.badge{font-size:.7rem;padding:.1rem .45rem;border-radius:99px;text-transform:uppercase;letter-spacing:.04em}
.badge.prod{background:#14532d;color:#86efac}
.badge.dev{background:#3b2f14;color:#fbbf24}
.new{font-size:.65rem;color:#0b1120;background:#4ade80;border-radius:99px;padding:.1rem .4rem;margin-left:.5rem;font-weight:700}
.meta{color:#64748b;font-size:.75rem;margin-top:.15rem}
a{color:#e2e8f0;text-decoration:none}a:hover{text-decoration:underline}
tr.warn td{background:#2a1e0a}tr.error td{background:#2a0f14}
.lvl{font-size:.7rem;padding:.05rem .4rem;border-radius:4px}
.lvl.info{color:#94a3b8}.lvl.warn{background:#3b2f14;color:#fbbf24}.lvl.error{background:#450a0a;color:#f87171}
.stream{color:#818cf8;font-size:.8em}
.back{color:#64748b;font-size:.85rem}
.health-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(13rem,1fr));gap:.7rem}
.health-card{border:1px solid #1e293b;border-radius:12px;background:#111a2d;padding:.8rem .9rem;min-height:8.5rem}
.health-card.healthy{border-color:#14532d}.health-card.recovered{border-color:#854d0e}
.health-card.degraded{border-color:#92400e}.health-card.action{border-color:#7f1d1d}
.health-head{display:flex;align-items:center;justify-content:space-between;gap:.5rem;margin-bottom:.55rem}
.health-area{color:#94a3b8;font-size:.72rem;font-weight:700;text-transform:uppercase;letter-spacing:.05em}
.health-card strong{display:block;font-size:.92rem}.health-card p{color:#cbd5e1;font-size:.82rem;margin:.3rem 0 .65rem}
.health-evidence{margin-top:.65rem}.health-evidence>summary{color:#93c5fd;cursor:pointer;font-size:.73rem}
.health-source{display:grid;gap:.35rem;margin-top:.45rem;max-height:14rem;overflow-x:hidden;overflow-y:auto}
.health-event{border-left:2px solid #334155;padding:.35rem .45rem}.health-event.warn{border-color:#854d0e}.health-event.error{border-color:#7f1d1d}
.health-event-head{align-items:center;display:flex;gap:.4rem;margin-bottom:.22rem}.health-event code{display:block;overflow-wrap:anywhere;white-space:normal;word-break:break-word}
.status{display:inline-block;border-radius:99px;font-size:.65rem;font-weight:750;letter-spacing:.035em;padding:.12rem .42rem;text-transform:uppercase;white-space:nowrap}
.status.healthy{background:#14532d;color:#86efac}.status.diagnostic{background:#1e293b;color:#94a3b8}
.status.recovered{background:#713f12;color:#fde68a}.status.degraded{background:#78350f;color:#fbbf24}
.status.action{background:#7f1d1d;color:#fecaca}
.problem-list{display:grid;gap:.55rem}
.problem-card{border:1px solid #1e293b;border-radius:10px;background:#111827;overflow:hidden}
.problem-card.action{border-color:#7f1d1d}.problem-card.degraded{border-color:#92400e}
.problem-card.recovered{border-color:#854d0e}
.problem-card summary{align-items:center;cursor:pointer;display:flex;justify-content:space-between;gap:1rem;list-style:none;padding:.75rem .85rem}
.problem-card summary::-webkit-details-marker{display:none}
.problem-card summary>span:first-child{align-items:center;display:flex;flex-wrap:wrap;gap:.55rem;min-width:0}
.problem-copy{color:#94a3b8;font-size:.8rem}.problem-count{color:#64748b;font-size:.76rem;white-space:nowrap}
.problem-detail{border-top:1px solid #1e293b;padding:.15rem .85rem .85rem}
.problem-detail p{font-size:.84rem;margin:.6rem 0}
.raw-timeline{border-top:1px solid #1e293b;margin-top:1.8rem;padding-top:.6rem}
.raw-timeline>summary{cursor:pointer;font-size:1rem;font-weight:700;margin:.6rem 0}
</style></head><body>%s</body></html>""" % body


def render_install(install_id, kind, n=200):
    n = max(50, min(n, 5000))
    fname = "events.dev.jsonl" if kind == "dev" else "events.jsonl"
    path = os.path.join(ROOT, install_id, fname)
    if not os.path.exists(path):
        return None
    meta = {}
    try:
        with open(os.path.join(ROOT, install_id, "meta.json")) as f:
            meta = json.load(f)
    except (OSError, ValueError):
        pass
    total = count_lines(path)
    size_mb = os.path.getsize(path) / 1048576
    events = []
    for line in tail_lines(path, n, max_bytes=max(512 * 1024, n * 600)):
        try:
            events.append(json.loads(line))
        except ValueError:
            continue

    title = meta.get("device_name") or install_id[:8]
    sub = " · ".join(
        x for x in (
            meta.get("os", ""),
            meta.get("specs", "") or meta.get("hw", ""),
            "v" + meta.get("last_version", "?"),
            kind,
            install_id,
        ) if x
    )
    state = {}
    try:
        with open(os.path.join(ROOT, install_id, "state.json")) as f:
            state = json.load(f)
    except (OSError, ValueError):
        pass

    signals = build_health_signals(events)
    health_cards = build_health_cards(signals, state)
    problem_groups = group_problem_signals(events, signals)
    health_html = (
        "<h2>Plain-English health</h2>"
        "<div class='health-grid'>%s</div>"
        "<p class='meta'>Based on the %d loaded events. Unknown events remain "
        "visible in the technical timeline and are never guessed.</p>"
        % ("".join(render_health_card(card) for card in health_cards), len(events))
    )

    info_html = ""
    if state:
        chips = []
        for ev in reversed(events):
            if str(ev.get("summary", "")).startswith("configure_dictation"):
                data = ev.get("data") or {}
                chips = ["%s: %s" % (k, v) for k, v in sorted(data.items())][:12]
                break
        if "default_input_available" in state:
            microphone = "Available" if state.get("default_input_available") else "Unavailable"
            input_count = state.get("input_device_count")
            inputs = (
                "%s detected%s"
                % (
                    input_count,
                    " (count capped)" if state.get("input_device_count_capped") else "",
                )
                if isinstance(input_count, int)
                else "unknown"
            )
            enumeration = (
                "Succeeded" if state.get("input_enumeration_ok") else "Failed"
            )
        else:
            microphone = state.get("default_input") or "unknown"
            others = [
                device
                for device in state.get("input_devices", [])
                if device != state.get("default_input")
            ]
            inputs = ", ".join(others) or "unknown"
            enumeration = "Legacy state snapshot"
        info_html = (
            '<h2>quick info</h2><table><tbody>'
            '<tr><td>Microphone</td><td><strong>%s</strong></td></tr>'
            '<tr><td>Inputs</td><td>%s</td></tr>'
            '<tr><td>Enumeration</td><td>%s</td></tr>'
            '<tr><td>Settings</td><td class="meta">%s</td></tr>'
            '<tr><td>As of</td><td>%s</td></tr>'
            '</tbody></table>'
            % (
                html.escape(str(microphone)),
                html.escape(str(inputs)),
                html.escape(str(enumeration)),
                html.escape(" · ".join(chips)) or "&mdash;",
                ago(state.get("received_at", 0)),
            )
        )

    attention_groups = [
        item for item in problem_groups if item["status"] in ("action", "degraded")
    ]
    explanation_groups = [
        item for item in problem_groups if item["status"] in ("recovered", "diagnostic")
    ]
    if attention_groups:
        attention_html = (
            "<h2>What needs attention</h2><div class='problem-list'>%s</div>"
            % "".join(render_problem_group(item) for item in attention_groups[:25])
        )
    else:
        attention_html = (
            "<h2>What needs attention</h2>"
            "<p class='sub'>No recognized problems in the loaded event window.</p>"
        )
    explanations_html = ""
    if explanation_groups:
        explanations_html = (
            "<h2>Recent explanations</h2>"
            "<p class='sub'>Grouped background signals and incidents that recovered "
            "without requiring action.</p><div class='problem-list'>%s</div>"
            % "".join(render_problem_group(item) for item in explanation_groups[:25])
        )
    base = "/install/%s" % install_id
    body = (
        '<p class="back"><a href="/">&larr; all devices</a></p>'
        "<h1>%s</h1><p class='sub'>%s</p>"
        "<p class='sub'>%s events on server (%.1f MB) &middot; "
        "show last <a href='%s?kind=%s&n=200'>200</a> / "
        "<a href='%s?kind=%s&n=1000'>1,000</a> / "
        "<a href='%s?kind=%s&n=5000'>5,000</a> &middot; "
        "<a href='%s/raw?kind=%s'>&darr; download entire log (.jsonl)</a></p>"
        "%s%s%s"
        "<details class='raw-timeline'><summary>Raw technical timeline "
        "(%d loaded events)</summary><table><tbody>%s</tbody></table></details>"
        % (
            html.escape(title),
            html.escape(sub),
            "{:,}".format(total),
            size_mb,
            base, kind, base, kind, base, kind, base, kind,
            health_html,
            info_html,
            attention_html + explanations_html,
            len(events),
            "".join(raw_event_row(event) for event in reversed(events)),
        )
    )
    page = render_dashboard()  # reuse the <style> shell
    return page[: page.index("<body>") + 6] + body + "</body></html>"


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "murmur-logs"

    def log_message(self, fmt, *args):
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))

    def _reply(self, code, msg=b"", ctype="text/plain"):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(msg)))
        self.end_headers()
        if msg:
            self.wfile.write(msg)

    def do_POST(self):
        if self.path == "/state":
            return self._do_state()
        if self.path != "/ingest":
            return self._reply(404)
        auth = self.headers.get("Authorization", "")
        if auth != "Bearer " + TOKEN:
            return self._reply(401)
        install_id = self.headers.get("X-Install-Id", "")
        if not INSTALL_ID_RE.match(install_id):
            return self._reply(400, b"bad install id")
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            return self._reply(400, b"bad length")
        if length <= 0:
            return self._reply(400, b"empty")
        if length > MAX_BODY:
            return self._reply(413)
        body = self.rfile.read(length)

        # Dev builds are not part of the fleet; ack so old debug builds
        # advance their offset, but store nothing.
        if self.headers.get("X-Dev") == "1":
            return self._reply(204)

        lines = []
        for raw in body.split(b"\n"):
            raw = raw.strip()
            if not raw:
                continue
            try:
                json.loads(raw)
            except ValueError:
                return self._reply(400, b"bad json line")
            lines.append(raw)
        if not lines:
            return self._reply(400, b"empty")

        suffix = ".dev" if self.headers.get("X-Dev") == "1" else ""
        dirpath = os.path.join(ROOT, install_id.lower())
        usage = root_usage()
        if usage["bytes"] > MAX_TOTAL:
            return self._reply(507, b"global quota exceeded")
        if not os.path.isdir(dirpath) and usage["dirs"] >= MAX_INSTALLS:
            return self._reply(507, b"install limit reached")
        os.makedirs(dirpath, exist_ok=True)
        path = os.path.join(dirpath, "events%s.jsonl" % suffix)
        try:
            if os.path.exists(path) and os.path.getsize(path) > MAX_FILE:
                return self._reply(507, b"install quota exceeded")
        except OSError:
            pass
        meta_path = os.path.join(dirpath, "meta.json")
        meta = {}
        try:
            with open(meta_path) as f:
                meta = json.load(f)
        except (OSError, ValueError):
            pass
        updates = {
            "last_version": self.headers.get("X-App-Version", ""),
            "device_name": self.headers.get("X-Device-Name", ""),
            "os": self.headers.get("X-Os-Version", ""),
            "hw": self.headers.get("X-Hw-Model", ""),
            "specs": self.headers.get("X-Hw-Specs", ""),
        }
        for k, v in updates.items():
            if v:
                if k == "device_name":
                    v = re.sub(r"(\w)\?(s\b)", r"\1'\2", v)
                meta[k] = v[:120]
        atomic_write_json(meta_path, meta)
        with open(path, "ab") as f:
            f.write(b"\n".join(lines) + b"\n")
        self._reply(204)

    def _do_state(self):
        if self.headers.get("Authorization", "") != "Bearer " + TOKEN:
            return self._reply(401)
        install_id = self.headers.get("X-Install-Id", "")
        if not INSTALL_ID_RE.match(install_id):
            return self._reply(400, b"bad install id")
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            return self._reply(400)
        if not 0 < length <= 32 * 1024:
            return self._reply(413)
        try:
            state = json.loads(self.rfile.read(length))
            assert isinstance(state, dict)
        except (ValueError, AssertionError):
            return self._reply(400, b"bad json")
        state["received_at"] = time.time()
        dirpath = os.path.join(ROOT, install_id.lower())
        usage = root_usage()
        if not os.path.isdir(dirpath) and usage["dirs"] >= MAX_INSTALLS:
            return self._reply(507, b"install limit reached")
        os.makedirs(dirpath, exist_ok=True)
        atomic_write_json(os.path.join(dirpath, "state.json"), state)
        self._reply(204)

    def do_GET(self):
        if self.path == "/healthz":
            return self._reply(200, b"ok")
        if self.path in ("/", "/dashboard"):
            page = render_dashboard().encode("utf-8")
            return self._reply(200, page, "text/html; charset=utf-8")
        if self.path.startswith("/install/"):
            rest = self.path[len("/install/"):]
            loc, _, query = rest.partition("?")
            kind = "dev" if "kind=dev" in query else "prod"
            install_id, _, sub = loc.partition("/")
            if not INSTALL_ID_RE.match(install_id):
                return self._reply(400, b"bad install id")
            install_id = install_id.lower()
            if sub == "raw":
                fname = "events.dev.jsonl" if kind == "dev" else "events.jsonl"
                path = os.path.join(ROOT, install_id, fname)
                if not os.path.exists(path):
                    return self._reply(404, b"no such install")
                size = os.path.getsize(path)
                self.send_response(200)
                self.send_header("Content-Type", "application/x-ndjson")
                self.send_header("Content-Length", str(size))
                self.send_header(
                    "Content-Disposition",
                    'attachment; filename="murmur-%s-%s.jsonl"' % (install_id[:8], kind),
                )
                self.end_headers()
                with open(path, "rb") as f:
                    while True:
                        chunk = f.read(1 << 20)
                        if not chunk:
                            break
                        self.wfile.write(chunk)
                return
            if sub:
                return self._reply(404)
            n = 200
            m = re.search(r"n=(\d+)", query)
            if m:
                n = int(m.group(1))
            page = render_install(install_id, kind, n)
            if page is None:
                return self._reply(404, b"no such install")
            return self._reply(200, page.encode("utf-8"), "text/html; charset=utf-8")
        self._reply(404)


def main():
    os.makedirs(ROOT, exist_ok=True)
    server = ThreadingHTTPServer(("127.0.0.1", 8600), Handler)
    sys.stderr.write("murmur-logs receiver on 127.0.0.1:8600\n")
    server.serve_forever()


if __name__ == "__main__":
    main()
