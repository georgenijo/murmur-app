#!/usr/bin/env python3
"""Scheduled capture-health and reliability-SLO watch for shipped Murmur logs.

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

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
if SCRIPT_DIR not in sys.path:
    sys.path.insert(0, SCRIPT_DIR)

from dictation_lifecycle import DictationLifecycleCorrelator, STAGE_CODES
from reliability_slo import ReliabilitySloEvaluator


SCHEMA_VERSION = 1
REPORT_NAME = "capture-watch.json"
MAX_STARTUP_SAMPLES = 500
MAX_POST_STOP_LATENCY_SAMPLES = 500
MAX_VERSIONS_PER_INSTALL = 64
MIN_COMPARISON_SAMPLES = 5
STARTUP_REGRESSION_RATIO = 2.0
REPEATED_ZERO_READY_SESSIONS = 2
RECENT_ATTEMPTED_SESSION_WINDOW = 5
MAX_READY_RECORDINGS_PER_SESSION = 20
CAPTURE_HEALTH_OWNER_KIND = "dictation"
PERFORMANCE_STORE_OPERATIONS = {
    "initialize",
    "begin",
    "update",
    "complete",
    "read",
    "write",
    "clear",
}
PERFORMANCE_STORE_ERROR_CLASSES = {
    "busyLocked",
    "storageFull",
    "readOnly",
    "io",
    "corruptIntegrity",
    "schemaMigration",
    "invalidRecord",
    "unavailable",
}

VERSION_RE = re.compile(r"^[0-9A-Za-z][0-9A-Za-z.+-]{0,39}$")
INSTALL_RE = re.compile(r"^[0-9a-fA-F-]{8,64}$")
BACKENDS = {"auhal", "cpal"}
SETUP_STEPS = {
    "none",
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


def bounded_performance_store_operation(value):
    return value if value in PERFORMANCE_STORE_OPERATIONS else "unknown"


def bounded_performance_store_error_class(value):
    return value if value in PERFORMANCE_STORE_ERROR_CLASSES else "unknown"


def numeric_startup(event):
    value = event_data(event).get("startup_ms")
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    if not math.isfinite(value) or value < 0 or value > 120_000:
        return None
    return float(value)


def numeric_post_stop_latency(event):
    value = event_data(event).get("total_ms")
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    if not math.isfinite(value) or value < 0 or value > 300_000:
        return None
    return float(value)


def valid_recording_id(value):
    if isinstance(value, bool) or not isinstance(value, int):
        return False
    return 0 < value < 2**64


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
        "post_stop_latency_samples": deque(maxlen=MAX_POST_STOP_LATENCY_SAMPLES),
        "post_stop_latency_sample_total": 0,
        "timeouts": Counter(),
        "fallback_count": 0,
        "both_backends_failed_count": 0,
        "performance_store_failures": Counter(),
        "attempted_sessions": 0,
        "recent_session_ready_counts": deque(maxlen=RECENT_ATTEMPTED_SESSION_WINDOW),
        "last_attempted_session_at": "",
        "first_event_at": "",
        "last_event_at": "",
        "dictation_lifecycle": DictationLifecycleCorrelator(),
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


def finish_session(session, cohorts, install_id, finished_at):
    if not session or not session["attempted"]:
        return
    cohort = cohort_for(cohorts, install_id, session["version"])
    cohort["attempted_sessions"] += 1
    cohort["recent_session_ready_counts"].append(session["ready_count"])
    if finished_at and finished_at > cohort["last_attempted_session_at"]:
        cohort["last_attempted_session_at"] = finished_at


def scan_install(path, install_id, cohorts, reliability_slo):
    malformed = 0
    session = None
    session_id = 0
    with open(path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            try:
                event = json.loads(line)
            except (ValueError, TypeError, RecursionError):
                malformed += 1
                reliability_slo.observe_malformed_source_line()
                continue
            if not isinstance(event, dict):
                malformed += 1
                reliability_slo.observe_malformed_source_line()
                continue

            code = event_code(event)
            data = event_data(event)
            owner_kind = data.get("owner_kind")
            # Shared microphone owners remain visible in retained raw logs,
            # but must not create, touch, or mutate dictation-health cohorts.
            # Keep owner-less pipeline/store events for their existing typed
            # dictation correlation while rejecting every explicitly scoped
            # non-dictation event before any watch state exists.
            if (
                owner_kind is not None
                and owner_kind != CAPTURE_HEALTH_OWNER_KIND
            ):
                continue

            # Feed the exact same dictation-or-unscoped record into the
            # aggregate SLO evaluator during this full scan. Install identity
            # is an internal correlation key only;
            # ReliabilitySloEvaluator.report() exposes no per-install rows or
            # identifiers. Explicitly non-dictation owners are rejected before
            # they can create even provisional request/state correlation.
            reliability_slo.observe(install_id, event)

            version = event_version(event)
            timestamp = event_timestamp(event)
            cohort = cohort_for(cohorts, install_id, version)
            touch_cohort(cohort, timestamp)

            if code == "system.startup_baseline" or event.get("summary") == "startup_baseline":
                for (cohort_install, _), existing in cohorts.items():
                    if cohort_install == install_id:
                        existing["dictation_lifecycle"].close_session(session_id)
                finish_session(session, cohorts, install_id, timestamp)
                session_id += 1
                session = {
                    "version": version,
                    "attempted": False,
                    "ready_count": 0,
                }
                continue

            if code in STAGE_CODES:
                # The receiver stamps versions per upload, so a delayed batch
                # can disagree with the version captured at startup. Never
                # split one recording across version cohorts.
                if session is not None and session["version"] == version:
                    cohort["dictation_lifecycle"].observe(event, session_id)
            if code == "pipeline.dictation_completed":
                # Same trusted install/app-session/version cohort boundaries as
                # the lifecycle report: a completion with no open session is
                # pre-baseline, and a version mismatch means the batch arrived
                # late for a session that has already closed. Both are
                # indistinguishable from a cross-session value and are
                # dropped rather than guessed.
                if (
                    session is not None
                    and session["version"] == version
                    and valid_recording_id(data.get("recording_id"))
                    and valid_recording_id(data.get("char_count"))
                ):
                    latency_ms = numeric_post_stop_latency(event)
                    if latency_ms is not None:
                        cohort["post_stop_latency_samples"].append(latency_ms)
                        cohort["post_stop_latency_sample_total"] += 1
                continue
            if code == "audio.capture_started":
                if (
                    data.get("owner_kind") == CAPTURE_HEALTH_OWNER_KIND
                    and session is not None
                    and session["version"] == version
                ):
                    session["attempted"] = True
                continue
            if code == "audio.capture_ready":
                if data.get("owner_kind") != CAPTURE_HEALTH_OWNER_KIND:
                    continue
                startup_ms = numeric_startup(event)
                if startup_ms is not None:
                    cohort["startup_samples"].append(startup_ms)
                    cohort["startup_sample_total"] += 1
                if session is not None and session["version"] == version:
                    session["ready_count"] = min(
                        session["ready_count"] + 1,
                        MAX_READY_RECORDINGS_PER_SESSION,
                    )
                continue
            if code == "audio.capture_backend_timeout":
                if data.get("owner_kind") != CAPTURE_HEALTH_OWNER_KIND:
                    continue
                key = (
                    bounded_backend(data.get("backend")),
                    bounded_setup_step(data.get("last_setup_step")),
                )
                cohort["timeouts"][key] += 1
                continue
            if code == "audio.fallback_started":
                if data.get("owner_kind") != CAPTURE_HEALTH_OWNER_KIND:
                    continue
                cohort["fallback_count"] += 1
                continue
            if code == "audio.capture_failed":
                if data.get("owner_kind") != CAPTURE_HEALTH_OWNER_KIND:
                    continue
                cohort["both_backends_failed_count"] += 1
                continue
            if code == "performance.store_operation_failed":
                key = (
                    bounded_performance_store_operation(data.get("operation")),
                    bounded_performance_store_error_class(data.get("error_class")),
                )
                cohort["performance_store_failures"][key] += 1

    # An open app session is deliberately not classified as zero-ready. A later
    # startup boundary proves that the preceding session ended and makes the
    # verdict deterministic.
    return malformed


def serialize_cohort(cohort):
    samples = list(cohort["startup_samples"])
    post_stop_latency_samples = list(cohort["post_stop_latency_samples"])
    recent_session_ready_counts = list(cohort["recent_session_ready_counts"])
    session_ready_histogram = Counter(recent_session_ready_counts)
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
    performance_store_failure_rows = [
        {
            "operation": operation,
            "error_class": error_class,
            "count": count,
        }
        for (operation, error_class), count in sorted(
            cohort["performance_store_failures"].items(),
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
        "post_stop_latency_sample_count": len(post_stop_latency_samples),
        "post_stop_latency_sample_total": cohort["post_stop_latency_sample_total"],
        "post_stop_latency_samples_truncated": (
            cohort["post_stop_latency_sample_total"] > len(post_stop_latency_samples)
        ),
        "post_stop_latency_p50_ms": percentile(post_stop_latency_samples, 50),
        "post_stop_latency_p95_ms": percentile(post_stop_latency_samples, 95),
        "capture_backend_timeouts": timeout_rows,
        "fallback_count": cohort["fallback_count"],
        "both_backends_failed_count": cohort["both_backends_failed_count"],
        "performance_store_failure_total": sum(
            cohort["performance_store_failures"].values()
        ),
        "performance_store_failures": performance_store_failure_rows,
        "attempted_sessions": cohort["attempted_sessions"],
        "evaluated_attempted_sessions": len(recent_session_ready_counts),
        "attempted_sessions_truncated": (
            cohort["attempted_sessions"] > len(recent_session_ready_counts)
        ),
        "last_attempted_session_at": cohort["last_attempted_session_at"],
        "zero_ready_sessions": sum(
            ready_count == 0 for ready_count in recent_session_ready_counts
        ),
        "ready_recordings_per_session": [
            {
                "ready_recordings": ready_count,
                "ready_recordings_capped": (
                    ready_count == MAX_READY_RECORDINGS_PER_SESSION
                ),
                "sessions": session_count,
            }
            for ready_count, session_count in sorted(
                session_ready_histogram.items()
            )
        ],
        "dictation_lifecycle": cohort["dictation_lifecycle"].report(),
    }


def regression_alerts(cohort_rows):
    alerts = []
    by_install = {}
    for row in cohort_rows:
        if row["app_version"] in ("unknown", "overflow"):
            continue
        by_install.setdefault(row["install_id"], []).append(row)

    for install_id, rows in by_install.items():
        latest = max(
            rows,
            key=lambda row: (row["last_event_at"], row["app_version"]),
        )
        for failure in latest["performance_store_failures"]:
            alerts.append(
                {
                    "kind": "performance_store_failure",
                    "install_id": install_id,
                    "app_version": latest["app_version"],
                    "operation": failure["operation"],
                    "error_class": failure["error_class"],
                    "count": failure["count"],
                }
            )
        lifecycle_rows = [
            row
            for row in rows
            if row["dictation_lifecycle"]["accepted"] > 0
            and row["last_event_at"]
        ]
        if lifecycle_rows:
            latest_lifecycle = max(
                lifecycle_rows,
                key=lambda row: (row["last_event_at"], row["app_version"]),
            )
            lifecycle = latest_lifecycle["dictation_lifecycle"]
            if lifecycle["missing_terminals"]:
                alerts.append(
                    {
                        "kind": "missing_dictation_terminals",
                        "install_id": install_id,
                        "app_version": latest_lifecycle["app_version"],
                        "count": lifecycle["missing_terminals"],
                    }
                )
            if lifecycle["duplicate_terminals"]:
                alerts.append(
                    {
                        "kind": "duplicate_dictation_terminals",
                        "install_id": install_id,
                        "app_version": latest_lifecycle["app_version"],
                        "count": lifecycle["duplicate_terminals"],
                    }
                )
        attempted_rows = [
            row
            for row in rows
            if row["evaluated_attempted_sessions"] > 0
            and row["last_attempted_session_at"]
        ]
        if attempted_rows:
            latest_attempted = max(
                attempted_rows,
                key=lambda row: (
                    row["last_attempted_session_at"],
                    row["last_event_at"],
                    row["app_version"],
                ),
            )
        else:
            latest_attempted = None
        if (
            latest_attempted is not None
            and latest_attempted["zero_ready_sessions"]
            >= REPEATED_ZERO_READY_SESSIONS
        ):
            alerts.append(
                {
                    "kind": "repeated_zero_ready_sessions",
                    "install_id": latest_attempted["install_id"],
                    "app_version": latest_attempted["app_version"],
                    "zero_ready_sessions": latest_attempted["zero_ready_sessions"],
                    "evaluated_attempted_sessions": latest_attempted[
                        "evaluated_attempted_sessions"
                    ],
                    "attempted_sessions": latest_attempted["attempted_sessions"],
                }
            )

        eligible = [
            row
            for row in rows
            if row["startup_sample_count"] >= MIN_COMPARISON_SAMPLES
            and row["startup_p50_ms"] is not None
        ]
        eligible.sort(key=lambda row: row["last_event_at"])
        if len(eligible) < 2:
            continue
        candidate = eligible[-1]
        baseline = min(
            eligible[:-1],
            key=lambda row: (row["startup_p50_ms"], row["last_event_at"]),
        )
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


def build_report(root, now=None):
    now = now or datetime.now(timezone.utc)
    reliability_evaluator = ReliabilitySloEvaluator(now=now, complete_weeks=8)
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
            malformed_lines += scan_install(
                path,
                install_id,
                cohorts,
                reliability_evaluator,
            )

    rows = [serialize_cohort(cohort) for cohort in cohorts.values()]
    rows.sort(key=lambda row: (row["install_id"], row["last_event_at"], row["app_version"]))
    alerts = regression_alerts(rows)
    reliability_slo = reliability_evaluator.report()
    newest_decisive_slo_week = next(
        (
            week
            for week in reliability_slo.get("weeks", [])
            if isinstance(week, dict)
            and week.get("complete") is True
            and week.get("verdict") != "insufficient"
        ),
        None,
    )
    reliability_alert = (
        reliability_slo.get("integrity", {}).get("status") == "indeterminate"
        or (
            isinstance(newest_decisive_slo_week, dict)
            and newest_decisive_slo_week.get("verdict") in ("fail", "indeterminate")
        )
    )
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
    status = (
        "alert"
        if alerts or reliability_alert
        else "healthy"
        if evaluable_rows
        else "insufficient_data"
    )
    return {
        "schema_version": SCHEMA_VERSION,
        "generated_at": now.isoformat().replace("+00:00", "Z"),
        "status": status,
        "scanned_installs": scanned_installs,
        "malformed_lines": malformed_lines,
        "policy": {
            "minimum_comparison_samples": MIN_COMPARISON_SAMPLES,
            "startup_p50_regression_ratio": STARTUP_REGRESSION_RATIO,
            "repeated_zero_ready_sessions": REPEATED_ZERO_READY_SESSIONS,
            "recent_attempted_session_window": RECENT_ATTEMPTED_SESSION_WINDOW,
            "maximum_ready_recordings_per_session": (
                MAX_READY_RECORDINGS_PER_SESSION
            ),
            "capture_health_owner_kind": CAPTURE_HEALTH_OWNER_KIND,
            "maximum_startup_samples_per_cohort": MAX_STARTUP_SAMPLES,
            "maximum_post_stop_latency_samples_per_cohort": (
                MAX_POST_STOP_LATENCY_SAMPLES
            ),
            "maximum_versions_per_install": MAX_VERSIONS_PER_INSTALL,
        },
        "alerts": alerts,
        "cohorts": rows,
        "reliability_slo": reliability_slo,
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
        help="exit 2 after writing the report when its status is alert",
    )
    args = parser.parse_args(argv)
    output = args.output or os.path.join(args.root, REPORT_NAME)
    report = build_report(args.root)
    atomic_write_report(output, report)
    decisive_slo_week = next(
        (
            week
            for week in report["reliability_slo"].get("weeks", [])
            if isinstance(week, dict)
            and week.get("complete") is True
            and week.get("verdict") != "insufficient"
        ),
        None,
    )
    slo_verdict = (
        "indeterminate"
        if report["reliability_slo"].get("integrity", {}).get("status")
        == "indeterminate"
        else decisive_slo_week.get("verdict")
        if isinstance(decisive_slo_week, dict)
        else "insufficient"
    )
    print(
        "capture watch: %s, %d install(s), %d cohort(s), "
        "%d legacy alert(s), reliability SLO %s"
        % (
            report["status"],
            report["scanned_installs"],
            len(report["cohorts"]),
            len(report["alerts"]),
            slo_verdict,
        )
    )
    return 2 if args.fail_on_alert and report["status"] == "alert" else 0


if __name__ == "__main__":
    sys.exit(main())
