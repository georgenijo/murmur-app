from __future__ import annotations

import importlib.util
from pathlib import Path
import unittest


MODULE_PATH = (
    Path(__file__).resolve().parents[1]
    / "infra"
    / "log-receiver"
    / "dictation_lifecycle.py"
)
SPEC = importlib.util.spec_from_file_location("dictation_lifecycle", MODULE_PATH)
assert SPEC and SPEC.loader
lifecycle = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(lifecycle)


def event(code: str, recording_id: int | None = None, **data: object) -> dict:
    fields = {"event_code": code, **data}
    if recording_id is not None:
        fields["recording_id"] = recording_id
    return {
        "timestamp": "2026-08-12T12:00:00Z",
        "stream": "pipeline",
        "level": "info",
        "summary": "constant",
        "data": fields,
    }


def audio_event(code: str, recording_id: int, owner_kind: str = "dictation") -> dict:
    item = event(
        code,
        recording_id,
        owner=recording_id,
        owner_kind=owner_kind,
    )
    item["stream"] = "audio"
    return item


class DictationLifecycleCorrelatorTests(unittest.TestCase):
    def test_reproduces_the_stage_funnel_and_terminal_outcomes(self) -> None:
        items = [event("system.startup_baseline")]
        for recording_id, outcome in enumerate(
            sorted(lifecycle.TERMINAL_OUTCOMES), start=1
        ):
            items.extend(
                [
                    event("pipeline.dictation_requested", recording_id),
                    audio_event("audio.capture_started", recording_id),
                    audio_event("audio.capture_ready", recording_id),
                    event("pipeline.dictation_stop_handoff", recording_id),
                    event(
                        "pipeline.dictation_terminal",
                        recording_id,
                        outcome=outcome,
                        error_code="none",
                    ),
                ]
            )
        items.append(event("system.startup_baseline"))

        report = lifecycle.correlate_events(items)

        expected = len(lifecycle.TERMINAL_OUTCOMES)
        self.assertEqual(report["requested"], expected)
        self.assertEqual(report["accepted"], expected)
        self.assertEqual(report["ready"], expected)
        self.assertEqual(report["stop_handoffs"], expected)
        self.assertEqual(report["terminal_attempts"], expected)
        self.assertEqual(report["outcomes"], {value: 1 for value in sorted(lifecycle.TERMINAL_OUTCOMES)})
        self.assertEqual(report["missing_terminals"], 0)
        self.assertEqual(report["duplicate_terminals"], 0)

    def test_out_of_order_events_correlate_without_timestamps_or_summaries(self) -> None:
        items = [
            event("system.startup_baseline"),
            event("pipeline.dictation_terminal", 9, outcome="success"),
            event("pipeline.dictation_stop_handoff", 9),
            audio_event("audio.capture_ready", 9),
            audio_event("audio.capture_started", 9),
            event("pipeline.dictation_requested", 9),
            event("system.startup_baseline"),
        ]
        for index, item in enumerate(items):
            item["timestamp"] = "not-used-%d" % (len(items) - index)
            item["summary"] = "untrusted-%d" % index

        report = lifecycle.correlate_events(items)

        self.assertEqual(report["drop_off"], {
            "request_to_accept": 0,
            "accept_to_ready": 0,
            "ready_to_stop_handoff": 0,
            "accepted_without_terminal": 0,
        })

    def test_closed_missing_and_duplicate_terminals_are_flagged(self) -> None:
        items = [
            event("system.startup_baseline"),
            audio_event("audio.capture_started", 1),
            audio_event("audio.capture_started", 2),
            event("pipeline.dictation_terminal", 2, outcome="success"),
            event("pipeline.dictation_terminal", 2, outcome="success"),
            event("system.startup_baseline"),
        ]

        report = lifecycle.correlate_events(items)

        self.assertEqual(report["missing_terminals"], 1)
        self.assertEqual(report["duplicate_terminals"], 1)
        self.assertEqual(report["terminal_events"], 2)
        self.assertEqual(report["outcomes"], {"success": 1})

    def test_duplicate_orphan_terminals_count_one_orphan_attempt(self) -> None:
        report = lifecycle.correlate_events(
            [
                event("system.startup_baseline"),
                event("pipeline.dictation_terminal", 8, outcome="success"),
                event("pipeline.dictation_terminal", 8, outcome="success"),
                event("pipeline.dictation_terminal", 8, outcome="success"),
                event("system.startup_baseline"),
            ]
        )
        self.assertEqual(report["orphan_stage_attempts"], 1)

    def test_open_app_session_is_not_a_missing_terminal(self) -> None:
        report = lifecycle.correlate_events(
            [
                event("system.startup_baseline"),
                audio_event("audio.capture_started", 1),
            ]
        )
        self.assertEqual(report["missing_terminals"], 0)
        self.assertEqual(report["open_accepted_without_terminal"], 1)

    def test_reused_recording_ids_stay_separate_across_app_sessions(self) -> None:
        report = lifecycle.correlate_events(
            [
                event("system.startup_baseline"),
                audio_event("audio.capture_started", 1),
                event("pipeline.dictation_terminal", 1, outcome="success"),
                event("system.startup_baseline"),
                audio_event("audio.capture_started", 1),
                event("pipeline.dictation_terminal", 1, outcome="no_speech"),
                event("system.startup_baseline"),
            ]
        )
        self.assertEqual(report["accepted"], 2)
        self.assertEqual(report["terminal_attempts"], 2)
        self.assertEqual(report["outcomes"], {"no_speech": 1, "success": 1})

    def test_transform_audio_never_enters_dictation_counts(self) -> None:
        report = lifecycle.correlate_events(
            [
                event("system.startup_baseline"),
                audio_event("audio.capture_started", 4, "transform"),
                audio_event("audio.capture_ready", 4, "transform"),
                event("system.startup_baseline"),
            ]
        )
        self.assertEqual(report["accepted"], 0)
        self.assertEqual(report["ready"], 0)

    def test_unknown_terminal_labels_are_counted_but_never_copied_to_report(self) -> None:
        report = lifecycle.correlate_events(
            [
                event("system.startup_baseline"),
                audio_event("audio.capture_started", 3),
                event(
                    "pipeline.dictation_terminal",
                    3,
                    outcome="private transcript content",
                ),
                event("system.startup_baseline"),
            ]
        )
        self.assertEqual(report["unknown_outcomes"], 1)
        self.assertNotIn("private transcript content", str(report))


if __name__ == "__main__":
    unittest.main()
