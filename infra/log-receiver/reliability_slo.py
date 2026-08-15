#!/usr/bin/env python3
"""Deterministic, aggregate-only evaluator for Murmur recording SLOs.

The evaluator consumes a full retained-log scan on every run. Install IDs are
used only as an in-memory correlation boundary and never enter the report.
Only dictation requests carrying the exact numeric ``slo_contract=1`` marker
are eligible; older telemetry cannot accidentally improve or degrade an SLO.
"""

from __future__ import annotations

import math
import re
from datetime import datetime, timedelta, timezone


SCHEMA_VERSION = 1
REPORT_FORMAT = "murmur-reliability-slo/v1"
CONTRACT_VERSION = 1
MIN_COMPLETE_WEEKS = 8
MIN_ELIGIBLE_REQUESTS = 200
STARTUP_TARGET_MS = 400.0
STARTUP_TARGET_FRACTION = 0.995
MAX_AGGREGATE_COUNT = 1_000_000_000
MAX_TRACKED_INSTALLS = 150
MAX_TRACKED_ATTEMPTS = 250_000
MAX_STATE_INTERVALS_PER_ATTEMPT = 8
MAX_SESSION_NUMBER = 1_000_000

STATE_NAMES = ("recovering", "processing")
STATE_VALUES = {"idle", "starting", "recording", *STATE_NAMES}
TERMINAL_OUTCOMES = {
    "success",
    "no_speech",
    "too_short",
    "user_cancelled_starting",
    "user_cancelled_recording",
    "user_cancelled_processing",
    "capture_init_failure",
    "runtime_interruption",
    "stop_failure",
    "pipeline_failure",
    "superseded",
}
CANCELLED_OUTCOMES = {
    "user_cancelled_starting",
    "user_cancelled_recording",
    "user_cancelled_processing",
    "superseded",
}
FAILURE_OUTCOMES = {
    "capture_init_failure",
    "runtime_interruption",
    "stop_failure",
    "pipeline_failure",
}

# Only these presentation pairs prove that a failure terminal received a
# useful action. Cleanup-in-progress is state guidance; a stop failure is
# covered only when the real stalled-cleanup UI was emitted successfully.
ACTIONABLE_PRESENTATIONS = {
    "capture_init_failure": {
        ("microphone_initialization_failed", "retry"),
        ("microphone_initialization_failed", "open_microphone_settings"),
        ("microphone_initialization_failed", "choose_microphone"),
    },
    "runtime_interruption": {
        ("microphone_interrupted", "retry"),
        ("microphone_interrupted", "wait_for_partial_transcription"),
    },
    "stop_failure": {
        ("microphone_cleanup_stalled", "restart_app"),
    },
}
KNOWN_PRESENTATIONS = {
    ("microphone_cleanup_in_progress", "wait"),
    ("microphone_initialization_failed", "retry"),
    ("microphone_initialization_failed", "open_microphone_settings"),
    ("microphone_initialization_failed", "choose_microphone"),
    ("microphone_cleanup_stalled", "restart_app"),
    ("microphone_interrupted", "retry"),
    ("microphone_interrupted", "wait_for_partial_transcription"),
}

UTC_TIMESTAMP_RE = re.compile(
    r"^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})"
    r"(?:\.(\d{1,9}))?Z$"
)


def _event_code(event):
    data = event.get("data") if isinstance(event, dict) else None
    if not isinstance(data, dict):
        return None
    code = data.get("event_code")
    return code if isinstance(code, str) else None


def _event_data(event):
    data = event.get("data") if isinstance(event, dict) else None
    return data if isinstance(data, dict) else {}


def _recording_id(data):
    value = data.get("recording_id")
    if isinstance(value, bool) or not isinstance(value, int):
        return None
    return value if 0 < value < 2**64 else None


def parse_utc_timestamp(value):
    """Parse the producer's strict UTC RFC3339 subset, never local offsets."""
    if not isinstance(value, str) or len(value) > 40:
        return None
    match = UTC_TIMESTAMP_RE.fullmatch(value)
    if match is None:
        return None
    year, month, day, hour, minute, second = map(int, match.groups()[:6])
    fraction = match.group(7) or ""
    microsecond = int((fraction + "000000")[:6])
    try:
        return datetime(
            year,
            month,
            day,
            hour,
            minute,
            second,
            microsecond,
            tzinfo=timezone.utc,
        )
    except ValueError:
        return None


def _strict_now(now):
    if now is None:
        return datetime.now(timezone.utc)
    if not isinstance(now, datetime) or now.tzinfo is None:
        raise ValueError("now must be an aware UTC datetime")
    if now.utcoffset() != timedelta(0):
        raise ValueError("now must use UTC")
    return now.astimezone(timezone.utc)


def _week_start(value):
    value = value.astimezone(timezone.utc)
    return (value - timedelta(days=value.weekday())).replace(
        hour=0, minute=0, second=0, microsecond=0
    )


def _format_timestamp(value):
    return value.astimezone(timezone.utc).isoformat(timespec="seconds").replace(
        "+00:00", "Z"
    )


def _nearest_rank(values, percentile):
    if not values:
        return None
    ordered = sorted(values)
    rank = max(1, math.ceil((percentile / 100.0) * len(ordered)))
    return ordered[min(rank - 1, len(ordered) - 1)]


def _duration_ms(start, end):
    if start is None or end is None or end < start:
        return None
    value = (end - start).total_seconds() * 1000.0
    # A corrupt timestamp must not create an unbounded numeric report value.
    return value if math.isfinite(value) and value <= 31 * 24 * 60 * 60 * 1000 else None


def _new_attempt():
    return {
        "request_count": 0,
        "requested_at": None,
        "accepted": False,
        "ready_at": None,
        "permission_pending": False,
        "terminal_count": 0,
        "terminal_outcome": None,
        "presentations": set(),
        "state_open": {name: None for name in STATE_NAMES},
        "state_intervals": {name: [] for name in STATE_NAMES},
        "state_anomalies": {name: 0 for name in STATE_NAMES},
        "state_cursor": None,
        "state_chain_anomalies": 0,
        "evidence_time_anomalies": 0,
    }


class ReliabilitySloEvaluator:
    """Full-scan evaluator with internal install/session/recording joins."""

    def __init__(self, now=None, complete_weeks=MIN_COMPLETE_WEEKS):
        if (
            isinstance(complete_weeks, bool)
            or not isinstance(complete_weeks, int)
            or complete_weeks != MIN_COMPLETE_WEEKS
        ):
            raise ValueError("complete_weeks must be exactly 8 for report contract v1")
        self.now = _strict_now(now)
        self.complete_weeks = complete_weeks
        self._install_sessions = {}
        self._attempts = {}
        self._session_attempt_keys = {}
        self._unassigned_contract_requests = 0
        self._overflowed_events = 0
        self._malformed_source_lines = 0

    def _mark_overflow(self):
        self._overflowed_events = min(
            self._overflowed_events + 1,
            MAX_AGGREGATE_COUNT,
        )

    def observe_malformed_source_line(self):
        """Fail closed when a retained source row cannot be evaluated."""
        self._malformed_source_lines = min(
            self._malformed_source_lines + 1,
            MAX_AGGREGATE_COUNT,
        )

    def _session(self, install_id):
        session = self._install_sessions.get(install_id)
        if session is not None:
            return session
        if len(self._install_sessions) >= MAX_TRACKED_INSTALLS:
            self._mark_overflow()
            return None
        session = {"active": False, "number": 0}
        self._install_sessions[install_id] = session
        return session

    def _attempt(self, install_id, session_number, recording_id):
        key = (install_id, session_number, recording_id)
        attempt = self._attempts.get(key)
        if attempt is not None:
            return attempt
        if len(self._attempts) >= MAX_TRACKED_ATTEMPTS:
            self._mark_overflow()
            return None
        attempt = _new_attempt()
        self._attempts[key] = attempt
        self._session_attempt_keys.setdefault(
            (install_id, session_number), set()
        ).add(key)
        return attempt

    def _append_state_interval(self, attempt, state, classification, duration):
        intervals = attempt["state_intervals"][state]
        if len(intervals) >= MAX_STATE_INTERVALS_PER_ATTEMPT:
            self._mark_overflow()
            attempt["state_anomalies"][state] = min(
                attempt["state_anomalies"][state] + 1,
                MAX_AGGREGATE_COUNT,
            )
            return
        intervals.append((classification, duration))

    def _close_for_restart(self, install_id, session_number, boundary_time):
        session_key = (install_id, session_number)
        for key in self._session_attempt_keys.get(session_key, ()):
            attempt = self._attempts.get(key)
            if attempt is None:
                continue
            for state in STATE_NAMES:
                started = attempt["state_open"][state]
                if started is not None:
                    duration = _duration_ms(started, boundary_time)
                    self._append_state_interval(
                        attempt,
                        state,
                        "restart_required" if duration is not None else "indeterminate",
                        duration,
                    )
                    attempt["state_open"][state] = None

    def _prune_closed_session(self, install_id, session_number):
        """Drop closed-session state that cannot enter the retained windows."""
        session_key = (install_id, session_number)
        keys = self._session_attempt_keys.pop(session_key, set())
        for key in keys:
            attempt = self._attempts.get(key)
            if attempt is None or attempt["requested_at"] is None:
                self._attempts.pop(key, None)

    def observe(self, install_id, event):
        """Observe one validated event in retained source order.

        ``install_id`` is deliberately opaque and used only in internal keys.
        Callers must perform a complete scan on every scheduled evaluation so
        late-arriving events can revise their original request week.
        """
        if (
            not isinstance(install_id, str)
            or not (1 <= len(install_id) <= 64)
            or not isinstance(event, dict)
        ):
            return
        code = _event_code(event)
        if code is None:
            return
        timestamp = parse_utc_timestamp(event.get("timestamp"))
        session = self._session(install_id)
        if session is None:
            return

        if code == "system.startup_baseline":
            if session["active"]:
                boundary_time = (
                    timestamp
                    if timestamp is not None and timestamp <= self.now
                    else None
                )
                self._close_for_restart(
                    install_id, session["number"], boundary_time
                )
                self._prune_closed_session(install_id, session["number"])
            if session["number"] >= MAX_SESSION_NUMBER:
                self._mark_overflow()
                session["active"] = False
                return
            session["number"] += 1
            session["active"] = True
            return
        if not session["active"]:
            return

        data = _event_data(event)
        rid = _recording_id(data)
        if rid is None:
            return
        relevant = {
            "pipeline.dictation_requested",
            "audio.capture_started",
            "audio.capture_ready",
            "audio.permission_prompt_changed",
            "pipeline.dictation_terminal",
            "pipeline.dictation_state_changed",
            "pipeline.dictation_presentation",
        }
        if code not in relevant:
            return

        if code in {
            "audio.capture_started",
            "audio.capture_ready",
            "audio.permission_prompt_changed",
        }:
            owner = data.get("owner")
            if (
                data.get("owner_kind") != "dictation"
                or isinstance(owner, bool)
                or not isinstance(owner, int)
                or owner != rid
            ):
                return

        earliest_retained = self._window_starts()[-1]
        if code == "pipeline.dictation_requested":
            contract = data.get("slo_contract")
            if (
                isinstance(contract, bool)
                or not isinstance(contract, int)
                or contract != CONTRACT_VERSION
            ):
                return
            if timestamp is None or timestamp > self.now:
                # There is no honest UTC week for this contract request. Keep
                # it out of every denominator, but surface bounded global
                # integrity evidence and refuse the two-week proof rather than
                # silently making the request disappear.
                self._unassigned_contract_requests = min(
                    self._unassigned_contract_requests + 1,
                    MAX_AGGREGATE_COUNT,
                )
                return
            if timestamp < earliest_retained:
                return
        elif timestamp is not None and timestamp < earliest_retained:
            # Events older than every published bucket cannot correlate to an
            # in-window producer attempt. Skip them before allocating state.
            return

        attempt = self._attempt(install_id, session["number"], rid)
        if attempt is None:
            return
        if code != "pipeline.dictation_requested" and (
            timestamp is None or timestamp > self.now
        ):
            # Non-request lifecycle evidence cannot prove a healthy outcome
            # before its own event time. Retain only a bounded contradiction
            # marker so a later/out-of-order in-window request fails closed.
            attempt["evidence_time_anomalies"] = min(
                attempt["evidence_time_anomalies"] + 1,
                MAX_AGGREGATE_COUNT,
            )
            return
        if code == "pipeline.dictation_requested":
            attempt["request_count"] = min(attempt["request_count"] + 1, 2)
            if attempt["requested_at"] is None or timestamp < attempt["requested_at"]:
                attempt["requested_at"] = timestamp
        elif code == "audio.capture_started":
            attempt["accepted"] = True
        elif code == "audio.capture_ready":
            if timestamp is not None and (
                attempt["ready_at"] is None or timestamp < attempt["ready_at"]
            ):
                attempt["ready_at"] = timestamp
        elif code == "audio.permission_prompt_changed":
            # Either retained half of the exact producer pair proves that the
            # attempt crossed the TCC prompt boundary. In particular, a
            # resolved record carries a validated bounded pending duration and
            # remains sufficient when retention or delivery omitted `pending`.
            if data.get("state") in ("pending", "resolved"):
                attempt["permission_pending"] = True
        elif code == "pipeline.dictation_terminal":
            outcome = data.get("outcome")
            attempt["terminal_count"] = min(attempt["terminal_count"] + 1, 2)
            if attempt["terminal_outcome"] is None:
                attempt["terminal_outcome"] = (
                    outcome if outcome in TERMINAL_OUTCOMES else "unknown"
                )
        elif code == "pipeline.dictation_presentation":
            pair = (data.get("status_code"), data.get("action_code"))
            if pair in KNOWN_PRESENTATIONS:
                attempt["presentations"].add(pair)
        elif code == "pipeline.dictation_state_changed":
            self._observe_state(attempt, data, timestamp)

    def _observe_state(self, attempt, data, timestamp):
        previous = data.get("from")
        current = data.get("to")
        if previous not in STATE_VALUES or current not in STATE_VALUES or previous == current:
            return
        cursor = attempt["state_cursor"]
        if cursor is not None and previous != cursor:
            attempt["state_chain_anomalies"] = min(
                attempt["state_chain_anomalies"] + 1,
                MAX_AGGREGATE_COUNT,
            )
            # Re-establish a bounded source-order cursor, but do not invent an
            # interval close/open from contradictory evidence.
            attempt["state_cursor"] = current
            return
        attempt["state_cursor"] = current
        for state in STATE_NAMES:
            if previous == state:
                started = attempt["state_open"][state]
                if started is None:
                    attempt["state_anomalies"][state] = min(
                        attempt["state_anomalies"][state] + 1,
                        MAX_AGGREGATE_COUNT,
                    )
                else:
                    duration = _duration_ms(started, timestamp)
                    if duration is None:
                        self._append_state_interval(
                            attempt, state, "indeterminate", None
                        )
                    else:
                        self._append_state_interval(
                            attempt, state, "self_recovered", duration
                        )
                    attempt["state_open"][state] = None
            if current == state:
                if attempt["state_open"][state] is not None:
                    attempt["state_anomalies"][state] = min(
                        attempt["state_anomalies"][state] + 1,
                        MAX_AGGREGATE_COUNT,
                    )
                else:
                    # ``None`` also represents an invalid timestamp. Count it
                    # now so the interval cannot disappear as apparently idle.
                    if timestamp is None:
                        attempt["state_anomalies"][state] = min(
                            attempt["state_anomalies"][state] + 1,
                            MAX_AGGREGATE_COUNT,
                        )
                    else:
                        attempt["state_open"][state] = timestamp

    def _window_starts(self):
        current = _week_start(self.now)
        return [current - timedelta(weeks=index) for index in range(self.complete_weeks + 1)]

    @staticmethod
    def _blank_week(start, current_start):
        return {
            "start": start,
            "end": start + timedelta(weeks=1),
            "complete": start < current_start,
            "attempts": [],
        }

    def _week_buckets(self):
        starts = self._window_starts()
        buckets = {
            start: self._blank_week(start, starts[0]) for start in starts
        }
        for attempt in self._attempts.values():
            if attempt["request_count"] <= 0 or attempt["requested_at"] is None:
                continue
            requested_at = attempt["requested_at"]
            start = _week_start(requested_at)
            if start in buckets:
                buckets[start]["attempts"].append(attempt)
        return [buckets[start] for start in starts]

    @staticmethod
    def _state_report(attempts, state):
        counts = {
            "self_recovered": 0,
            "restart_required": 0,
            "indeterminate": 0,
        }
        durations = []
        for attempt in attempts:
            for classification, duration in attempt["state_intervals"][state]:
                counts[classification] = min(
                    counts[classification] + 1,
                    MAX_AGGREGATE_COUNT,
                )
                if duration is not None:
                    durations.append(duration)
            counts["indeterminate"] = min(
                counts["indeterminate"] + attempt["state_anomalies"][state],
                MAX_AGGREGATE_COUNT,
            )
            if attempt["state_open"][state] is not None:
                counts["indeterminate"] = min(
                    counts["indeterminate"] + 1,
                    MAX_AGGREGATE_COUNT,
                )
        return {
            **counts,
            "duration_ms": {
                "sample_count": len(durations),
                "p50": _nearest_rank(durations, 50),
                "p95": _nearest_rank(durations, 95),
                "max": max(durations) if durations else None,
            },
        }

    @staticmethod
    def _actionable(attempt, outcome):
        allowed = ACTIONABLE_PRESENTATIONS.get(outcome, set())
        return bool(allowed.intersection(attempt["presentations"]))

    def _render_week(self, bucket):
        attempts = bucket["attempts"]
        excluded = [item for item in attempts if item["permission_pending"]]
        eligible = [item for item in attempts if not item["permission_pending"]]
        counts = {
            "requested": len(attempts),
            "eligible_requests": len(eligible),
            "excluded_permission_prompts": len(excluded),
            "accepted": 0,
            "ready": 0,
            "ready_without_accepted": 0,
            "within_400": 0,
            "failed": 0,
            "cancelled": 0,
            "missing_terminals": 0,
            "duplicate_terminals": 0,
            "unknown_terminals": 0,
            "failures_with_actionable_presentation": 0,
            "failures_without_actionable_presentation": 0,
            "duplicate_requests": 0,
            "invalid_startup_timings": 0,
            "invalid_state_transitions": min(
                sum(item["state_chain_anomalies"] for item in attempts),
                MAX_AGGREGATE_COUNT,
            ),
            "invalid_evidence_timestamps": min(
                sum(item["evidence_time_anomalies"] for item in attempts),
                MAX_AGGREGATE_COUNT,
            ),
        }
        startup_samples = []
        # Funnel, terminal-integrity, state, and presentation evidence cover
        # every v1 request. Permission-pending excludes only the latency SLO.
        for attempt in attempts:
            counts["accepted"] += bool(attempt["accepted"])
            counts["duplicate_requests"] += attempt["request_count"] > 1
            terminal_count = attempt["terminal_count"]
            if terminal_count == 0:
                counts["missing_terminals"] += 1
            if terminal_count > 1:
                counts["duplicate_terminals"] += 1
            outcome = attempt["terminal_outcome"]
            if outcome == "unknown":
                counts["unknown_terminals"] += 1
            elif outcome in CANCELLED_OUTCOMES:
                counts["cancelled"] += 1
            elif outcome in FAILURE_OUTCOMES:
                counts["failed"] += 1
                if self._actionable(attempt, outcome):
                    counts["failures_with_actionable_presentation"] += 1
                else:
                    counts["failures_without_actionable_presentation"] += 1

            if attempt["ready_at"] is not None:
                counts["ready"] += 1
                if not attempt["accepted"]:
                    counts["ready_without_accepted"] += 1

        for attempt in eligible:
            if attempt["ready_at"] is not None:
                requested_at = attempt["requested_at"]
                ready_at = attempt["ready_at"]
                latency = _duration_ms(requested_at, ready_at)
                if latency is None:
                    counts["invalid_startup_timings"] += 1
                else:
                    startup_samples.append(latency)
                    counts["within_400"] += latency <= STARTUP_TARGET_MS

        state_report = {
            state: self._state_report(attempts, state) for state in STATE_NAMES
        }
        indeterminate = []
        if counts["missing_terminals"]:
            indeterminate.append("missing_terminal")
        if counts["duplicate_terminals"]:
            indeterminate.append("duplicate_terminal")
        if counts["unknown_terminals"]:
            indeterminate.append("unknown_terminal")
        if counts["duplicate_requests"]:
            indeterminate.append("duplicate_request")
        if counts["invalid_startup_timings"]:
            indeterminate.append("invalid_startup_timing")
        if counts["ready_without_accepted"]:
            indeterminate.append("ready_without_accepted")
        if counts["invalid_state_transitions"]:
            indeterminate.append("invalid_state_transition")
        if counts["invalid_evidence_timestamps"]:
            indeterminate.append("invalid_evidence_timestamp")
        if any(value["indeterminate"] for value in state_report.values()):
            indeterminate.append("state_interval_indeterminate")

        failures = []
        if any(value["restart_required"] for value in state_report.values()):
            failures.append("restart_required_state")
        fraction = (
            counts["within_400"] / counts["eligible_requests"]
            if counts["eligible_requests"]
            else None
        )
        if fraction is not None and fraction < STARTUP_TARGET_FRACTION:
            failures.append("startup_target_missed")
        if counts["failures_without_actionable_presentation"]:
            failures.append("failure_presentation_missing")

        sample_status = (
            "partial"
            if not bucket["complete"]
            else (
                "below_minimum"
                if counts["eligible_requests"] < MIN_ELIGIBLE_REQUESTS
                else "sufficient"
            )
        )
        if not bucket["complete"]:
            verdict = "insufficient"
            # Keep bounded evidence visible without allowing a partial window
            # to claim a definitive weekly SLO result.
            reasons = ["partial_week", *indeterminate, *failures]
        elif indeterminate:
            verdict = "indeterminate"
            reasons = indeterminate
        elif failures:
            verdict = "fail"
            reasons = failures
        elif counts["eligible_requests"] < MIN_ELIGIBLE_REQUESTS:
            verdict = "insufficient"
            reasons = ["eligible_requests_below_minimum"]
        else:
            verdict = "pass"
            reasons = []

        return {
            "week_start": _format_timestamp(bucket["start"]),
            "week_end": _format_timestamp(bucket["end"]),
            "complete": bucket["complete"],
            "sample_status": sample_status,
            "verdict": verdict,
            "reasons": reasons,
            "counts": counts,
            "startup_ms": {
                "sample_count": len(startup_samples),
                "p50": _nearest_rank(startup_samples, 50),
                "p95": _nearest_rank(startup_samples, 95),
                "max": max(startup_samples) if startup_samples else None,
                "within_400_fraction": fraction,
            },
            "states": state_report,
        }

    def report(self):
        weeks = [self._render_week(bucket) for bucket in self._week_buckets()]
        latest_complete = [week for week in weeks if week["complete"]][:2]
        consecutive_pass = (
            len(latest_complete) == 2
            and all(week["verdict"] == "pass" for week in latest_complete)
            and self._unassigned_contract_requests == 0
            and self._overflowed_events == 0
            and self._malformed_source_lines == 0
        )
        return {
            "schema_version": SCHEMA_VERSION,
            "report": REPORT_FORMAT,
            "generated_at": _format_timestamp(self.now),
            "contract_version": CONTRACT_VERSION,
            "privacy": "aggregate_only",
            "integrity": {
                "status": (
                    "complete"
                    if self._unassigned_contract_requests == 0
                    and self._overflowed_events == 0
                    and self._malformed_source_lines == 0
                    else "indeterminate"
                ),
                "unassigned_contract_requests": self._unassigned_contract_requests,
                "overflowed_events": self._overflowed_events,
                "malformed_source_lines": self._malformed_source_lines,
            },
            "thresholds": {
                "complete_weeks": self.complete_weeks,
                "minimum_eligible_requests": MIN_ELIGIBLE_REQUESTS,
                "startup_target_ms": STARTUP_TARGET_MS,
                "startup_target_fraction": STARTUP_TARGET_FRACTION,
            },
            "two_consecutive_complete_weeks_pass": consecutive_pass,
            "weeks": weeks,
        }


def evaluate_fleet(event_feed, now=None, complete_weeks=MIN_COMPLETE_WEEKS):
    """Convenience API for an iterable of ``(install_id, event)`` records."""
    evaluator = ReliabilitySloEvaluator(now=now, complete_weeks=complete_weeks)
    for install_id, event in event_feed:
        evaluator.observe(install_id, event)
    return evaluator.report()
