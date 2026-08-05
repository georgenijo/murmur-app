from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


RECEIVER_PATH = (
    Path(__file__).resolve().parents[1]
    / "infra"
    / "log-receiver"
    / "murmur-logs-receiver.py"
)
SPEC = importlib.util.spec_from_file_location("murmur_logs_receiver", RECEIVER_PATH)
assert SPEC and SPEC.loader
receiver = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(receiver)


def event(
    summary: str,
    *,
    timestamp: str,
    level: str = "info",
    stream: str = "system",
    data: dict | None = None,
) -> dict:
    return {
        "timestamp": timestamp,
        "stream": stream,
        "level": level,
        "summary": summary,
        "data": data or {},
    }


class LogReceiverHealthTests(unittest.TestCase):
    def test_stable_event_code_takes_precedence_over_compatibility_summary(self) -> None:
        item = event(
            "listener heartbeat — no rdev callbacks observed",
            timestamp="2026-08-05T00:00:00Z",
            level="warn",
            stream="keyboard",
            data={"event_code": "keyboard.listener_failed"},
        )

        self.assertEqual(receiver.event_code(item), "keyboard.listener_failed")
        self.assertEqual(receiver.classify_event(item)["status"], "action")

    def test_listener_silence_is_diagnostic_and_deduplicated(self) -> None:
        events = [
            event(
                "listener heartbeat — no rdev callbacks observed",
                timestamp=f"2026-08-05T00:{minute:02d}:00Z",
                level="warn",
                stream="keyboard",
                data={
                    "silent_for_ms": 300_000 + minute * 60_000,
                    "threshold_ms": 300_000,
                },
            )
            for minute in range(20)
        ]

        signals = receiver.build_health_signals(events)
        groups = receiver.group_problem_signals(events, signals)

        self.assertEqual(len(groups), 1)
        self.assertEqual(groups[0]["status"], "diagnostic")
        self.assertEqual(groups[0]["count"], 20)
        self.assertEqual(groups[0]["title"], "No recent shortcut activity")

    def test_fallback_then_readiness_is_one_recovered_incident(self) -> None:
        events = [
            event(
                "capture backend exceeded its active initialization budget",
                timestamp="2026-08-05T00:00:00Z",
                level="warn",
                stream="audio",
                data={
                    "owner": 44,
                    "backend": "auhal",
                    "capture_id": 10,
                    "last_setup_step": "stream_start",
                },
            ),
            event(
                "capture backend failed before retained audio; trying bounded fallback",
                timestamp="2026-08-05T00:00:01Z",
                level="warn",
                stream="audio",
                data={
                    "owner": 44,
                    "from_backend": "auhal",
                    "to_backend": "cpal",
                },
            ),
            event(
                "audio readiness accepted",
                timestamp="2026-08-05T00:00:02Z",
                stream="audio",
                data={"owner": 44, "startup_ms": 8_400},
            ),
        ]

        signals = receiver.build_health_signals(events)
        groups = receiver.group_problem_signals(events, signals)
        cards = receiver.build_health_cards(signals, {})

        self.assertEqual(len(signals), 1)
        self.assertEqual(signals[0]["code"], "audio.fallback_recovered")
        self.assertEqual(signals[0]["status"], "recovered")
        self.assertEqual(len(signals[0]["events"]), 3)
        self.assertEqual(len(groups), 1)
        microphone = next(card for card in cards if card["area"] == "microphone")
        self.assertEqual(microphone["status"], "recovered")

    def test_interleaved_owners_keep_recovery_evidence_scoped(self) -> None:
        events = [
            event(
                "capture backend exceeded its active initialization budget",
                timestamp="2026-08-05T00:00:00Z",
                level="warn",
                stream="audio",
                data={"owner": 44, "backend": "auhal"},
            ),
            event(
                "capture backend exceeded its active initialization budget",
                timestamp="2026-08-05T00:00:01Z",
                level="warn",
                stream="audio",
                data={"owner": 55, "backend": "auhal"},
            ),
            event(
                "capture backend failed before retained audio; trying bounded fallback",
                timestamp="2026-08-05T00:00:02Z",
                level="warn",
                stream="audio",
                data={
                    "owner": 55,
                    "from_backend": "auhal",
                    "to_backend": "cpal",
                },
            ),
            event(
                "capture backend failed before retained audio; trying bounded fallback",
                timestamp="2026-08-05T00:00:03Z",
                level="warn",
                stream="audio",
                data={
                    "owner": 44,
                    "from_backend": "auhal",
                    "to_backend": "cpal",
                },
            ),
            event(
                "audio readiness accepted",
                timestamp="2026-08-05T00:00:04Z",
                stream="audio",
                data={"owner": 44, "startup_ms": 900},
            ),
            event(
                "audio readiness accepted",
                timestamp="2026-08-05T00:00:05Z",
                stream="audio",
                data={"owner": 55, "startup_ms": 1_100},
            ),
        ]

        recovered = [
            item
            for item in receiver.build_health_signals(events)
            if item["code"] == "audio.fallback_recovered"
        ]

        self.assertEqual(len(recovered), 2)
        self.assertEqual(
            [
                {receiver.event_value(source, "owner") for source in item["events"]}
                for item in recovered
            ],
            [{44}, {55}],
        )

    def test_exhausted_fallback_is_one_actionable_failure(self) -> None:
        events = [
            event(
                "capture backend failed before retained audio; trying bounded fallback",
                timestamp="2026-08-05T00:00:00Z",
                level="warn",
                stream="audio",
                data={
                    "owner": 45,
                    "from_backend": "auhal",
                    "to_backend": "cpal",
                },
            ),
            event(
                "both capture backend attempts failed before first PCM",
                timestamp="2026-08-05T00:00:01Z",
                level="error",
                stream="audio",
                data={"owner": 45, "error_kind": "initialization_timeout"},
            ),
            event(
                "audio lifecycle failed",
                timestamp="2026-08-05T00:00:02Z",
                level="error",
                stream="audio",
                data={"owner": 45, "error_kind": "initialization_timeout"},
            ),
        ]

        signals = receiver.build_health_signals(events)
        groups = receiver.group_problem_signals(events, signals)

        self.assertEqual(len(signals), 1)
        self.assertEqual(signals[0]["status"], "action")
        self.assertEqual(len(signals[0]["events"]), 3)
        self.assertEqual(len(groups), 1)
        self.assertEqual(groups[0]["count"], 1)
        self.assertEqual(groups[0]["title"], "Microphone failed")

    def test_unknown_error_outranks_noisy_diagnostic_without_a_guess(self) -> None:
        events = [
            event(
                "listener heartbeat — no rdev callbacks observed",
                timestamp="2026-08-05T00:00:00Z",
                level="warn",
                stream="keyboard",
            ),
            event(
                "<unsafe> opaque failure",
                timestamp="2026-08-05T00:01:00Z",
                level="error",
                stream="system",
                data={"detail": "<private>"},
            ),
        ]

        signals = receiver.build_health_signals(events)
        groups = receiver.group_problem_signals(events, signals)

        self.assertEqual(groups[0]["status"], "action")
        self.assertEqual(groups[0]["title"], "Technical error")
        self.assertIn("no safe plain-English mapping", groups[0]["action"])

    def test_data_labels_are_bounded_and_unknown_outcomes_share_one_group(self) -> None:
        backend = "unsafe\n" + "x" * 200
        timeout = event(
            "capture backend exceeded its active initialization budget",
            timestamp="2026-08-05T00:00:00Z",
            level="warn",
            stream="audio",
            data={"owner": 1, "backend": backend},
        )
        transform_events = [
            event(
                "transform_pass_outcome",
                timestamp=f"2026-08-05T00:00:0{second}Z",
                stream="transform",
                data={"outcome": "unmapped-" + str(second) + "-" + "y" * 200},
            )
            for second in (1, 2)
        ]

        timeout_signal = receiver.classify_event(timeout)
        groups = receiver.group_problem_signals(
            transform_events,
            receiver.build_health_signals(transform_events),
        )

        self.assertNotIn("\n", timeout_signal["explanation"])
        self.assertLess(len(timeout_signal["explanation"]), 100)
        self.assertEqual(len(groups), 1)
        self.assertEqual(groups[0]["group"], "transform.pass_outcome.other")
        self.assertEqual(groups[0]["count"], 2)

    def test_newer_microphone_state_overrides_older_ready_event(self) -> None:
        ready = event(
            "audio readiness accepted",
            timestamp="2026-08-05T00:00:00Z",
            stream="audio",
            data={"owner": 44, "startup_ms": 900},
        )
        state = {
            "received_at": receiver.event_epoch(ready) + 1,
            "default_input_available": False,
            "input_enumeration_ok": True,
        }

        cards = receiver.build_health_cards(
            receiver.build_health_signals([ready]),
            state,
        )
        microphone = next(card for card in cards if card["area"] == "microphone")

        self.assertEqual(microphone["status"], "action")
        self.assertEqual(microphone["title"], "No default microphone available")

        state["received_at"] = receiver.event_epoch(ready) - 1
        older_state_cards = receiver.build_health_cards(
            receiver.build_health_signals([ready]),
            state,
        )
        older_state_microphone = next(
            card for card in older_state_cards if card["area"] == "microphone"
        )
        self.assertEqual(older_state_microphone["status"], "healthy")

    def test_render_install_escapes_unknown_content_and_keeps_raw_timeline(self) -> None:
        install_id = "12345678-abcd"
        events = [
            event(
                "<script>alert('summary')</script>",
                timestamp="2026-08-05T00:00:00Z",
                level="error",
                stream="<audio>",
                data={"detail": "<img src=x>"},
            ),
            event(
                "listener heartbeat — no rdev callbacks observed",
                timestamp="2026-08-05T00:01:00Z",
                level="warn",
                stream="keyboard",
            ),
        ]
        with tempfile.TemporaryDirectory() as directory:
            original_root = receiver.ROOT
            receiver.ROOT = directory
            try:
                install_dir = Path(directory) / install_id
                install_dir.mkdir()
                (install_dir / "events.jsonl").write_text(
                    "\n".join(json.dumps(item) for item in events) + "\n",
                    encoding="utf-8",
                )
                (install_dir / "meta.json").write_text(
                    json.dumps({"device_name": "<Murmur Mac>", "last_version": "1.0"}),
                    encoding="utf-8",
                )
                page = receiver.render_install(install_id, "prod", 200)
            finally:
                receiver.ROOT = original_root

        self.assertIsNotNone(page)
        self.assertIn("Plain-English health", page)
        self.assertIn("What needs attention", page)
        self.assertIn("Recent explanations", page)
        self.assertIn('<span class="status action">Action</span>', page)
        self.assertIn("Technical details", page)
        self.assertIn("Raw technical timeline", page)
        self.assertIn("&lt;script&gt;", page)
        self.assertIn("&lt;Murmur Mac&gt;", page)
        self.assertNotIn("<script>", page)
        self.assertNotIn("<img src=x>", page)


if __name__ == "__main__":
    unittest.main()
