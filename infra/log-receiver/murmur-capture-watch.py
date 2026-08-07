#!/usr/bin/env python3
"""Scheduled capture-startup regression watch for shipped Murmur logs.

The watch reads the already privacy-stripped per-install JSONL retained by the
log receiver. It never reads diagnostic bundles, transcript history, audio, or
device identifiers. Reports are written atomically so the dashboard never sees
a partial run.
"""

import argparse
import json
import math
import os
import re
import sys
import tempfile
from collections import Counter, deque
from datetime import datetime, timezone


SCHEMA_VERSION = 1
REPORT_NAME = "capture-watch.json"
MAX_STARTUP_SAMPLES = 500
MAX_VERSIONS_PER_INSTALL = 64
MIN_COMPARISON_SAMPLES = 5
STARTUP_REGRESSION_RATIO = 2.0
REPEATED_ZERO_READY_SESSIONS = 2

VERSION_RE = re.compile(r"^[0-9A-Za-z][0-9A-Za-z.+-]{0,39}$")
INSTALL_RE = re.compile(r"^[0-9a-fA-F-]{8,64}$")
BACKENDS = {"auhal", "cpal"}
SETUP_STEPS = {
    "device_resolution",
    "audio_unit_creation",
    "audio_unit_new",
    "enable_input_io",
    "disable_output_io",
    "set_current_device",
    "format_configuration",
    "callback_installation",
    "default_config",
    "stream_build",
    "stream_start",
    "awaiting_first_callback",
}

SUMMARY_CODES = {
    "audio initialization accepted": "audio.capture_started",
    "capture backend exceeded its active initialization budget": (
        "audio.capture_backend_timeout"
    ),
    "capture backend failed before retained audio; trying bounded fallback": (
        "audio.fallback_started"
    ),
    "audio readiness accepted": "audio.capture_ready",
    "both capture backend attempts failed before first PCM": "audio.capture_failed",
}


def event_code(event):
    data = event.get("data")
    if isinstance(data, dict):
        code = data.get("event_code")
        if isinstance(code, str):
            return code
    return SUMMARY_CODES.get(event.get("summary"))


def event_data(event):
    data = event.get("data")
    return data if isinstance(data, dict) else {}


def event_version(event):
    value = event.get("ingest_app_version")
    return value if isinstance(value, str) and VERSION_RE.fullmatch(value) else "unknown"


def event_timestamp(event):
    value = event.get("timestamp")
    return value if isinstance(value, str) and len(value) <= 40 else ""


def bounded_backend(value):
    return value if value in BACKENDS else "unknown"


def bounded_setup_step(value):
    return value if value in SETUP_STEPS else "unknown"


def numeric_startup(event):
    value = event_data(event).get("startup_ms")
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    if not math.isfinite(value) or value < 0 or value > 120_000:
        return None
    return float(value)


def percentile(values, percentile_value):
    """Nearest-rank percentile, stable for small bounded operational cohorts."""
    if not values:
        return None
    ordered = sorted(values)
    rank = max(1, math.ceil((percentile_value / 100) * len(ordered)))
    return ordered[min(rank - 1, len(ordered) - 1)]


def new_cohort(install_id, version):
    return {
        "install_id": install_id,
        "app_version": version,
        "startup_samples": deque(maxlen=MAX_STARTUP_SAMPLES),
        "startup_sample_total": 0,
        "timeouts": Counter(),
        "fallback_count": 0,
        "both_backends_failed_count": 0,
        "attempted_sessions": 0,
        "zero_ready_sessions": 0,
        "session_ready_histogram": Counter(),
        "first_event_at": "",
        "last_event_at": "",
    }


def cohort_for(cohorts, install_id, version):
    key = (install_id, version)
    if key not in cohorts and version not in ("unknown", "overflow"):
        known_versions = sum(
            1
            for cohort_install, cohort_version in cohorts
            if cohort_install == install_id
            and cohort_version not in ("unknown", "overflow")
        )
        if known_versions >= MAX_VERSIONS_PER_INSTALL:
            version = "overflow"
    key = (install_id, version)
    if key not in cohorts:
        cohorts[key] = new_cohort(install_id, version)
    return cohorts[key]


def touch_cohort(cohort, timestamp):
    if not timestamp:
        return
    if not cohort["first_event_at"] or timestamp < cohort["first_event_at"]:
        cohort["first_event_at"] = timestamp
    if not cohort["last_event_at"] or timestamp > cohort["last_event_at"]:
        cohort["last_event_at"] = timestamp


def finish_session(session, cohorts, install_id):
    if not session or not session["attempted"]:
        return
    cohort = cohort_for(cohorts, install_id, session["version"])
    cohort["attempted_sessions"] += 1
    cohort["session_ready_histogram"][session["ready_count"]] += 1
    if session["ready_count"] == 0:
        cohort["zero_ready_sessions"] += 1


def scan_install(path, install_id, cohorts):
    malformed = 0
    session = None
    with open(path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            try:
                event = json.loads(line)
            except (ValueError, TypeError):
                malformed += 1
                continue
            if not isinstance(event, dict):
                malformed += 1
                continue

            version = event_version(event)
            timestamp = event_timestamp(event)
            cohort = cohort_for(cohorts, install_id, version)
            touch_cohort(cohort, timestamp)

            if event.get("summary") == "startup_baseline":
                finish_session(session, cohorts, install_id)
                session = {
                    "version": version,
                    "attempted": False,
                    "ready_count": 0,
                }
                continue

            code = event_code(event)
            data = event_data(event)
            if code == "audio.capture_started":
                if session is not None and session["version"] == version:
                    session["attempted"] = True
                continue
            if code == "audio.capture_ready":
                startup_ms = numeric_startup(event)
                if startup_ms is not None:
                    cohort["startup_samples"].append(startup_ms)
                    cohort["startup_sample_total"] += 1
                if session is not None and session["version"] == version:
                    session["ready_count"] += 1
                continue
            if code == "audio.capture_backend_timeout":
                key = (
                    bounded_backend(data.get("backend")),
                    bounded_setup_step(data.get("last_setup_step")),
                )
                cohort["timeouts"][key] += 1
                continue
            if code == "audio.fallback_started":
                cohort["fallback_count"] += 1
                continue
            if code == "audio.capture_failed":
                cohort["both_backends_failed_count"] += 1

    # An open app session is deliberately not classified as zero-ready. A later
    # startup boundary proves that the preceding session ended and makes the
    # verdict deterministic.
    return malformed


def serialize_cohort(cohort):
    samples = list(cohort["startup_samples"])
    timeout_rows = [
        {
            "backend": backend,
            "last_setup_step": setup_step,
            "count": count,
        }
        for (backend, setup_step), count in sorted(
            cohort["timeouts"].items(),
            key=lambda item: (-item[1], item[0]),
        )
    ]
    return {
        "install_id": cohort["install_id"],
        "app_version": cohort["app_version"],
        "first_event_at": cohort["first_event_at"],
        "last_event_at": cohort["last_event_at"],
        "startup_sample_count": len(samples),
        "startup_sample_total": cohort["startup_sample_total"],
        "startup_samples_truncated": cohort["startup_sample_total"] > len(samples),
        "startup_p50_ms": percentile(samples, 50),
        "startup_p95_ms": percentile(samples, 95),
        "capture_backend_timeouts": timeout_rows,
        "fallback_count": cohort["fallback_count"],
        "both_backends_failed_count": cohort["both_backends_failed_count"],
        "attempted_sessions": cohort["attempted_sessions"],
        "zero_ready_sessions": cohort["zero_ready_sessions"],
        "ready_recordings_per_session": [
            {"ready_recordings": ready_count, "sessions": session_count}
            for ready_count, session_count in sorted(
                cohort["session_ready_histogram"].items()
            )
        ],
    }


def regression_alerts(cohort_rows):
    alerts = []
    by_install = {}
    for row in cohort_rows:
        if row["app_version"] in ("unknown", "overflow"):
            continue
        by_install.setdefault(row["install_id"], []).append(row)

        if row["zero_ready_sessions"] >= REPEATED_ZERO_READY_SESSIONS:
            alerts.append(
                {
                    "kind": "repeated_zero_ready_sessions",
                    "install_id": row["install_id"],
                    "app_version": row["app_version"],
                    "zero_ready_sessions": row["zero_ready_sessions"],
                    "attempted_sessions": row["attempted_sessions"],
                }
            )

    for install_id, rows in by_install.items():
        eligible = [
            row
            for row in rows
            if row["startup_sample_count"] >= MIN_COMPARISON_SAMPLES
            and row["startup_p50_ms"] is not None
        ]
        eligible.sort(key=lambda row: row["last_event_at"])
        if len(eligible) < 2:
            continue
        baseline, candidate = eligible[-2:]
        baseline_p50 = baseline["startup_p50_ms"]
        candidate_p50 = candidate["startup_p50_ms"]
        if baseline_p50 > 0 and candidate_p50 > baseline_p50 * STARTUP_REGRESSION_RATIO:
            alerts.append(
                {
                    "kind": "startup_p50_regression",
                    "install_id": install_id,
                    "baseline_version": baseline["app_version"],
                    "candidate_version": candidate["app_version"],
                    "baseline_p50_ms": baseline_p50,
                    "candidate_p50_ms": candidate_p50,
                    "ratio": round(candidate_p50 / baseline_p50, 3),
                    "baseline_samples": baseline["startup_sample_count"],
                    "candidate_samples": candidate["startup_sample_count"],
                }
            )
    return alerts


def build_report(root):
    cohorts = {}
    malformed_lines = 0
    scanned_installs = 0
    if os.path.isdir(root):
        for install_id in sorted(os.listdir(root)):
            if not INSTALL_RE.fullmatch(install_id):
                continue
            path = os.path.join(root, install_id, "events.jsonl")
            if not os.path.isfile(path):
                continue
            scanned_installs += 1
            malformed_lines += scan_install(path, install_id, cohorts)

    rows = [serialize_cohort(cohort) for cohort in cohorts.values()]
    rows.sort(key=lambda row: (row["install_id"], row["last_event_at"], row["app_version"]))
    alerts = regression_alerts(rows)
    known_rows = [
        row
        for row in rows
        if row["app_version"] not in ("unknown", "overflow")
    ]
    evaluable_rows = [
        row
        for row in known_rows
        if row["startup_sample_count"] >= MIN_COMPARISON_SAMPLES
        or row["attempted_sessions"] >= REPEATED_ZERO_READY_SESSIONS
    ]
    status = "alert" if alerts else "healthy" if evaluable_rows else "insufficient_data"
    return {
        "schema_version": SCHEMA_VERSION,
        "generated_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "status": status,
        "scanned_installs": scanned_installs,
        "malformed_lines": malformed_lines,
        "policy": {
            "minimum_comparison_samples": MIN_COMPARISON_SAMPLES,
            "startup_p50_regression_ratio": STARTUP_REGRESSION_RATIO,
            "repeated_zero_ready_sessions": REPEATED_ZERO_READY_SESSIONS,
            "maximum_startup_samples_per_cohort": MAX_STARTUP_SAMPLES,
            "maximum_versions_per_install": MAX_VERSIONS_PER_INSTALL,
        },
        "alerts": alerts,
        "cohorts": rows,
    }


def atomic_write_report(path, report):
    directory = os.path.dirname(os.path.abspath(path))
    os.makedirs(directory, exist_ok=True)
    descriptor, temporary = tempfile.mkstemp(prefix=".capture-watch-", dir=directory)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(report, handle, sort_keys=True, separators=(",", ":"))
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        default=os.path.expanduser("~/murmur-logs"),
        help="receiver log root",
    )
    parser.add_argument(
        "--output",
        help="report path (defaults to <root>/%s)" % REPORT_NAME,
    )
    parser.add_argument(
        "--fail-on-alert",
        action="store_true",
        help="exit 2 after writing the report when alerts are present",
    )
    args = parser.parse_args(argv)
    output = args.output or os.path.join(args.root, REPORT_NAME)
    report = build_report(args.root)
    atomic_write_report(output, report)
    print(
        "capture watch: %s, %d install(s), %d cohort(s), %d alert(s)"
        % (
            report["status"],
            report["scanned_installs"],
            len(report["cohorts"]),
            len(report["alerts"]),
        )
    )
    return 2 if args.fail_on_alert and report["alerts"] else 0


if __name__ == "__main__":
    sys.exit(main())
