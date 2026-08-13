"""Deterministic, content-free correlator for Murmur dictation lifecycle events."""

from collections import Counter


STAGE_CODES = {
    "pipeline.dictation_requested": "requested",
    "audio.capture_started": "accepted",
    "audio.capture_ready": "ready",
    "pipeline.dictation_stop_handoff": "stop_handoff",
    "pipeline.dictation_terminal": "terminal",
}

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


def stable_event_code(event):
    data = event.get("data")
    if not isinstance(data, dict):
        return None
    code = data.get("event_code")
    return code if isinstance(code, str) else None


def recording_id(event, code):
    data = event.get("data")
    if not isinstance(data, dict):
        return None
    if code in ("audio.capture_started", "audio.capture_ready"):
        if data.get("owner_kind") != "dictation":
            return None
    value = data.get("recording_id")
    if isinstance(value, bool) or not isinstance(value, int):
        return None
    return value if 0 < value < 2**64 else None


class DictationLifecycleCorrelator:
    def __init__(self):
        self._attempts = {}
        self._closed_totals = Counter()

    def observe(self, event, session_id):
        code = stable_event_code(event)
        stage = STAGE_CODES.get(code)
        if stage is None:
            return
        rid = recording_id(event, code)
        if rid is None:
            return
        key = (session_id, rid)
        attempt = self._attempts.setdefault(
            key,
            {
                "requested": False,
                "accepted": False,
                "ready": False,
                "stop_handoff": False,
                "terminal_count": 0,
                "terminal_outcome": None,
            },
        )
        if stage == "terminal":
            if attempt["terminal_count"] == 0:
                data = event.get("data", {})
                outcome = data.get("outcome")
                attempt["terminal_outcome"] = (
                    outcome if outcome in TERMINAL_OUTCOMES else "unknown"
                )
            attempt["terminal_count"] += 1
        else:
            attempt[stage] = True

    def close_session(self, session_id):
        keys = [key for key in self._attempts if key[0] == session_id]
        closed = [self._attempts.pop(key) for key in keys]
        self._closed_totals.update(self._attempt_counts(closed, closed=True))

    @staticmethod
    def _attempt_counts(attempts, closed):
        counts = Counter()
        for item in attempts:
            requested = item["requested"]
            accepted = item["accepted"]
            ready = accepted and item["ready"]
            handoff = accepted and item["stop_handoff"]
            terminal_count = item["terminal_count"]
            counts["requested"] += requested
            counts["accepted"] += accepted
            counts["ready"] += ready
            counts["stop_handoffs"] += handoff
            counts["terminal_attempts"] += accepted and terminal_count >= 1
            counts["terminal_events"] += terminal_count
            if accepted and terminal_count >= 1:
                terminal_outcome = item["terminal_outcome"]
                if terminal_outcome in TERMINAL_OUTCOMES:
                    counts["outcome:%s" % terminal_outcome] += 1
                else:
                    counts["unknown_outcomes"] += 1
            counts["missing_terminals"] += accepted and closed and terminal_count == 0
            counts["open_accepted_without_terminal"] += (
                accepted and not closed and terminal_count == 0
            )
            counts["duplicate_terminals"] += accepted and terminal_count > 1
            counts["orphan_stage_attempts"] += (
                not accepted
                and (item["ready"] or item["stop_handoff"] or terminal_count)
            )
            counts["accepted_without_request"] += accepted and not requested
            counts["request_to_accept"] += requested and not accepted
            counts["accept_to_ready"] += accepted and not item["ready"]
            counts["ready_to_stop_handoff"] += accepted and item["ready"] and not handoff
        return counts

    def report(self):
        totals = self._closed_totals + self._attempt_counts(
            self._attempts.values(), closed=False
        )
        return {
            "requested": totals["requested"],
            "accepted": totals["accepted"],
            "ready": totals["ready"],
            "stop_handoffs": totals["stop_handoffs"],
            "terminal_attempts": totals["terminal_attempts"],
            "terminal_events": totals["terminal_events"],
            "outcomes": {
                outcome: totals["outcome:%s" % outcome]
                for outcome in sorted(TERMINAL_OUTCOMES)
                if totals["outcome:%s" % outcome]
            },
            "unknown_outcomes": totals["unknown_outcomes"],
            "missing_terminals": totals["missing_terminals"],
            "open_accepted_without_terminal": totals[
                "open_accepted_without_terminal"
            ],
            "duplicate_terminals": totals["duplicate_terminals"],
            "orphan_stage_attempts": totals["orphan_stage_attempts"],
            "accepted_without_request": totals["accepted_without_request"],
            "drop_off": {
                "request_to_accept": totals["request_to_accept"],
                "accept_to_ready": totals["accept_to_ready"],
                "ready_to_stop_handoff": totals["ready_to_stop_handoff"],
                "accepted_without_terminal": totals["missing_terminals"],
            },
        }


def correlate_events(events):
    correlator = DictationLifecycleCorrelator()
    session_id = 0
    for event in events:
        if stable_event_code(event) == "system.startup_baseline":
            correlator.close_session(session_id)
            session_id += 1
            continue
        correlator.observe(event, session_id)
    return correlator.report()
