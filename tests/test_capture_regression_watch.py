from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


WATCH_PATH = (
    Path(__file__).resolve().parents[1]
    / "infra"
    / "log-receiver"
    / "murmur-capture-watch.py"
)
SPEC = importlib.util.spec_from_file_location("murmur_capture_watch", WATCH_PATH)
assert SPEC and SPEC.loader
watch = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(watch)


def event(
    summary: str,
    timestamp: str,
    *,
    version: str | None = None,
    data: dict | None = None,
) -> dict:
    item = {
        "timestamp": timestamp,
        "stream": "audio",
        "level": "info",
        "summary": summary,
        "data": data or {},
    }
    if version is not None:
        item["ingest_app_version"] = version
    return item


class CaptureRegressionWatchTests(unittest.TestCase):
    def write_install(self, root: str, install_id: str, events: list[dict]) -> None:
        directory = Path(root) / install_id
        directory.mkdir()
        (directory / "events.jsonl").write_text(
            "\n".join(json.dumps(item) for item in events) + "\n",
            encoding="utf-8",
        )

    def ready_events(
        self,
        version: str,
        prefix: str,
        startup_values: list[int],
    ) -> list[dict]:
        return [
            event(
                "audio readiness accepted",
                f"2026-08-{prefix}T00:00:{index:02d}Z",
                version=version,
                data={
                    "event_code": "audio.capture_ready",
                    "owner_kind": "dictation",
                    "startup_ms": value,
                },
            )
            for index, value in enumerate(startup_values)
        ]

    def test_aggregates_percentiles_timeouts_fallbacks_and_failures(self) -> None:
        install_id = "12345678-abcd"
        events = self.ready_events("1.2.3", "01", [100, 200, 300, 400, 500])
        events.extend(
            [
                event(
                    "capture backend exceeded its active initialization budget",
                    "2026-08-01T00:01:00Z",
                    version="1.2.3",
                    data={
                        "event_code": "audio.capture_backend_timeout",
                        "owner_kind": "dictation",
                        "backend": "auhal",
                        "last_setup_step": "stream_start",
                    },
                ),
                event(
                    "capture backend failed before retained audio; trying bounded fallback",
                    "2026-08-01T00:01:01Z",
                    version="1.2.3",
                    data={
                        "event_code": "audio.fallback_started",
                        "owner_kind": "dictation",
                    },
                ),
                event(
                    "both capture backend attempts failed before first PCM",
                    "2026-08-01T00:01:02Z",
                    version="1.2.3",
                    data={
                        "event_code": "audio.capture_failed",
                        "owner_kind": "dictation",
                    },
                ),
            ]
        )
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, install_id, events)
            report = watch.build_report(root)

        cohort = report["cohorts"][0]
        self.assertEqual(cohort["startup_p50_ms"], 300)
        self.assertEqual(cohort["startup_p95_ms"], 500)
        self.assertEqual(cohort["fallback_count"], 1)
        self.assertEqual(cohort["both_backends_failed_count"], 1)
        self.assertEqual(
            cohort["capture_backend_timeouts"],
            [{"backend": "auhal", "last_setup_step": "stream_start", "count": 1}],
        )

    def test_microphone_benchmark_events_do_not_enter_dictation_health_counts(self) -> None:
        baseline_timestamp = "2026-08-01T00:00:00Z"
        events = [
            event(
                "startup_baseline",
                baseline_timestamp,
                version="1.2.3",
                data={"event_code": "system.startup_baseline"},
            ),
            event(
                "audio initialization accepted",
                "2026-08-02T00:00:00Z",
                version="9.9.9",
                data={
                    "event_code": "audio.capture_started",
                    "owner": 100,
                    "owner_kind": "microphone_benchmark",
                },
            ),
            event(
                "audio readiness accepted",
                "2026-08-02T00:00:01Z",
                version="9.9.9",
                data={
                    "event_code": "audio.capture_ready",
                    "owner": 100,
                    "owner_kind": "microphone_benchmark",
                    "startup_ms": 42,
                },
            ),
        ]
        for cycle in range(5):
            owner = cycle + 1
            events.extend(
                [
                    event(
                        "audio initialization accepted",
                        f"2026-08-02T00:00:{cycle * 3:02d}Z",
                        version="9.9.9",
                        data={
                            "event_code": "audio.capture_started",
                            "owner": owner,
                            "owner_kind": "microphone_benchmark",
                        },
                    ),
                    event(
                        "capture backend exceeded its active initialization budget",
                        f"2026-08-02T00:01:{cycle * 3:02d}Z",
                        version="9.9.9",
                        data={
                            "event_code": "audio.capture_backend_timeout",
                            "owner": owner,
                            "owner_kind": "microphone_benchmark",
                            "backend": "auhal",
                            "last_setup_step": "stream_start",
                        },
                    ),
                    event(
                        "capture backend failed before retained audio; trying bounded fallback",
                        f"2026-08-02T00:01:{cycle * 3 + 1:02d}Z",
                        version="9.9.9",
                        data={
                            "event_code": "audio.fallback_started",
                            "owner": owner,
                            "owner_kind": "microphone_benchmark",
                        },
                    ),
                    event(
                        "both capture backend attempts failed before first PCM",
                        f"2026-08-02T00:01:{cycle * 3 + 2:02d}Z",
                        version="9.9.9",
                        data={
                            "event_code": "audio.capture_failed",
                            "owner": owner,
                            "owner_kind": "microphone_benchmark",
                        },
                    ),
                ]
            )
        events.extend(
            [
                event(
                    "ignored benchmark-shaped pipeline request",
                    "2026-08-02T00:02:00Z",
                    version="9.9.9",
                    data={
                        "event_code": "pipeline.dictation_requested",
                        "owner_kind": "microphone_benchmark",
                        "recording_id": 91,
                    },
                ),
                event(
                    "ignored benchmark-shaped pipeline terminal",
                    "2026-08-02T00:02:01Z",
                    version="9.9.9",
                    data={
                        "event_code": "pipeline.dictation_terminal",
                        "owner_kind": "microphone_benchmark",
                        "recording_id": 91,
                        "outcome": "success",
                    },
                ),
                event(
                    "ignored benchmark-shaped store failure",
                    "2026-08-02T00:02:02Z",
                    version="9.9.9",
                    data={
                        "event_code": "performance.store_operation_failed",
                        "owner_kind": "microphone_benchmark",
                        "operation": "begin",
                        "error_class": "busyLocked",
                    },
                ),
            ]
        )

        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        cohort = report["cohorts"][0]
        self.assertEqual(len(report["cohorts"]), 1)
        self.assertEqual(cohort["app_version"], "1.2.3")
        self.assertEqual(cohort["first_event_at"], baseline_timestamp)
        self.assertEqual(cohort["last_event_at"], baseline_timestamp)
        self.assertEqual(cohort["startup_sample_count"], 0)
        self.assertEqual(cohort["capture_backend_timeouts"], [])
        self.assertEqual(cohort["fallback_count"], 0)
        self.assertEqual(cohort["both_backends_failed_count"], 0)
        self.assertEqual(cohort["performance_store_failure_total"], 0)
        self.assertEqual(cohort["attempted_sessions"], 0)
        self.assertEqual(cohort["dictation_lifecycle"]["requested"], 0)
        self.assertEqual(cohort["dictation_lifecycle"]["terminal_events"], 0)

    def test_performance_store_failures_are_counted_by_version_and_safe_class(self) -> None:
        events = [
            event(
                "untrusted diagnostic text",
                "2026-08-01T00:00:00Z",
                version="1.2.3",
                data={
                    "event_code": "performance.store_operation_failed",
                    "operation": "begin",
                    "error_class": "busyLocked",
                    "attempts": 3,
                    "recording_id": 35,
                },
            ),
            event(
                "another untrusted summary",
                "2026-08-01T00:00:01Z",
                version="1.2.3",
                data={
                    "event_code": "performance.store_operation_failed",
                    "operation": "begin",
                    "error_class": "busyLocked",
                    "attempts": 3,
                    "recording_id": 36,
                },
            ),
            event(
                "ignored",
                "2026-08-02T00:00:00Z",
                version="1.2.4",
                data={
                    "event_code": "performance.store_operation_failed",
                    "operation": "begin",
                    "error_class": "readOnly",
                    "attempts": 1,
                    "recording_id": 1,
                },
            ),
        ]
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        cohorts = {row["app_version"]: row for row in report["cohorts"]}
        self.assertEqual(cohorts["1.2.3"]["performance_store_failure_total"], 2)
        self.assertEqual(
            cohorts["1.2.3"]["performance_store_failures"],
            [{"operation": "begin", "error_class": "busyLocked", "count": 2}],
        )
        self.assertEqual(cohorts["1.2.4"]["performance_store_failure_total"], 1)
        self.assertEqual(report["status"], "alert")
        self.assertEqual(
            report["alerts"],
            [
                {
                    "kind": "performance_store_failure",
                    "install_id": "12345678-abcd",
                    "app_version": "1.2.4",
                    "operation": "begin",
                    "error_class": "readOnly",
                    "count": 1,
                }
            ],
        )

    def test_performance_store_watch_collapses_untrusted_labels(self) -> None:
        events = [
            event(
                "ignored",
                "2026-08-01T00:00:00Z",
                version="1.2.3",
                data={
                    "event_code": "performance.store_operation_failed",
                    "operation": "SELECT private_path",
                    "error_class": "/Users/private/diagnostics.sqlite3",
                },
            )
        ]
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        self.assertEqual(
            report["cohorts"][0]["performance_store_failures"],
            [{"operation": "unknown", "error_class": "unknown", "count": 1}],
        )
        encoded = json.dumps(report)
        self.assertNotIn("private_path", encoded)
        self.assertNotIn("/Users/private", encoded)

    def test_alerts_when_newest_version_p50_is_more_than_twice_baseline(self) -> None:
        events = self.ready_events("1.0.0", "01", [180, 190, 200, 210, 220])
        events += self.ready_events("1.1.0", "02", [500, 520, 540, 560, 580])
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        self.assertEqual(report["status"], "alert")
        self.assertEqual(len(report["alerts"]), 1)
        alert = report["alerts"][0]
        self.assertEqual(alert["kind"], "startup_p50_regression")
        self.assertEqual(alert["baseline_version"], "1.0.0")
        self.assertEqual(alert["candidate_version"], "1.1.0")
        self.assertEqual(alert["baseline_p50_ms"], 200)
        self.assertEqual(alert["candidate_p50_ms"], 540)

    def test_persistent_p50_regression_uses_best_retained_earlier_baseline(self) -> None:
        events = self.ready_events("1.0.0", "01", [200] * 5)
        events += self.ready_events("1.1.0", "02", [500] * 5)
        events += self.ready_events("1.2.0", "03", [520] * 5)
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        alert = report["alerts"][0]
        self.assertEqual(alert["kind"], "startup_p50_regression")
        self.assertEqual(alert["baseline_version"], "1.0.0")
        self.assertEqual(alert["candidate_version"], "1.2.0")
        self.assertEqual(alert["ratio"], 2.6)

    def test_small_or_cross_install_cohorts_never_form_a_regression(self) -> None:
        with tempfile.TemporaryDirectory() as root:
            self.write_install(
                root,
                "12345678-abcd",
                self.ready_events("1.0.0", "01", [100, 100, 100, 100]),
            )
            self.write_install(
                root,
                "87654321-abcd",
                self.ready_events("1.1.0", "02", [900, 900, 900, 900, 900]),
            )
            report = watch.build_report(root)

        self.assertEqual(report["alerts"], [])
        self.assertEqual(report["status"], "healthy")

    def test_repeated_completed_attempted_sessions_without_ready_alert(self) -> None:
        version = "1.2.3"
        events = []
        for day in ("01", "02", "03"):
            events.extend(
                [
                    event("startup_baseline", f"2026-08-{day}T00:00:00Z", version=version),
                    event(
                        "audio initialization accepted",
                        f"2026-08-{day}T00:00:01Z",
                        version=version,
                        data={"owner_kind": "dictation"},
                    ),
                ]
            )
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        cohort = report["cohorts"][0]
        self.assertEqual(cohort["attempted_sessions"], 2)
        self.assertEqual(cohort["zero_ready_sessions"], 2)
        self.assertEqual(
            cohort["ready_recordings_per_session"],
            [
                {
                    "ready_recordings": 0,
                    "ready_recordings_capped": False,
                    "sessions": 2,
                }
            ],
        )
        self.assertEqual(report["alerts"][0]["kind"], "repeated_zero_ready_sessions")

    def test_recent_healthy_sessions_clear_zero_ready_alert(self) -> None:
        version = "1.2.3"
        events = []
        for day in range(1, 8):
            events.append(
                event(
                    "startup_baseline",
                    f"2026-08-{day:02d}T00:00:00Z",
                    version=version,
                )
            )
            events.append(
                event(
                    "audio initialization accepted",
                    f"2026-08-{day:02d}T00:00:01Z",
                    version=version,
                    data={"owner_kind": "dictation"},
                )
            )
            if day >= 3:
                events.append(
                    event(
                        "audio readiness accepted",
                        f"2026-08-{day:02d}T00:00:02Z",
                        version=version,
                        data={
                            "event_code": "audio.capture_ready",
                            "owner_kind": "dictation",
                            "startup_ms": 200,
                        },
                    )
                )
        events.append(
            event("startup_baseline", "2026-08-08T00:00:00Z", version=version)
        )
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        cohort = report["cohorts"][0]
        self.assertEqual(cohort["attempted_sessions"], 7)
        self.assertEqual(cohort["evaluated_attempted_sessions"], 5)
        self.assertTrue(cohort["attempted_sessions_truncated"])
        self.assertEqual(cohort["zero_ready_sessions"], 0)
        self.assertEqual(report["alerts"], [])
        self.assertEqual(report["status"], "healthy")

    def test_healthy_attempt_on_new_version_supersedes_stale_zero_ready_cohort(self) -> None:
        events = [
            event("startup_baseline", "2026-08-01T00:00:00Z", version="1.0.0"),
            event(
                "audio initialization accepted",
                "2026-08-01T00:00:01Z",
                version="1.0.0",
                data={"owner_kind": "dictation"},
            ),
            event("startup_baseline", "2026-08-02T00:00:00Z", version="1.0.0"),
            event(
                "audio initialization accepted",
                "2026-08-02T00:00:01Z",
                version="1.0.0",
                data={"owner_kind": "dictation"},
            ),
            event("startup_baseline", "2026-08-03T00:00:00Z", version="1.1.0"),
            event(
                "audio initialization accepted",
                "2026-08-03T00:00:01Z",
                version="1.1.0",
                data={"owner_kind": "dictation"},
            ),
            event(
                "audio readiness accepted",
                "2026-08-03T00:00:02Z",
                version="1.1.0",
                data={
                    "event_code": "audio.capture_ready",
                    "owner_kind": "dictation",
                    "startup_ms": 200,
                },
            ),
            event("startup_baseline", "2026-08-04T00:00:00Z", version="1.1.0"),
        ]
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        cohorts = {row["app_version"]: row for row in report["cohorts"]}
        self.assertEqual(cohorts["1.0.0"]["zero_ready_sessions"], 2)
        self.assertEqual(cohorts["1.1.0"]["zero_ready_sessions"], 0)
        self.assertEqual(report["alerts"], [])

    def test_idle_and_still_open_sessions_are_not_zero_ready(self) -> None:
        version = "1.2.3"
        events = [
            event("startup_baseline", "2026-08-01T00:00:00Z", version=version),
            event("heartbeat", "2026-08-01T00:01:00Z", version=version),
            event("startup_baseline", "2026-08-02T00:00:00Z", version=version),
            event(
                "audio initialization accepted",
                "2026-08-02T00:00:01Z",
                version=version,
                data={"owner_kind": "dictation"},
            ),
        ]
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        cohort = report["cohorts"][0]
        self.assertEqual(cohort["attempted_sessions"], 0)
        self.assertEqual(cohort["zero_ready_sessions"], 0)
        self.assertEqual(report["alerts"], [])
        self.assertEqual(report["status"], "insufficient_data")

    def test_historical_unversioned_events_remain_unknown_and_noncomparable(self) -> None:
        events = self.ready_events("1.2.3", "02", [900, 900, 900, 900, 900])
        events += [
            event(
                "audio readiness accepted",
                f"2026-08-01T00:00:0{index}Z",
                data={"owner_kind": "dictation", "startup_ms": 100},
            )
            for index in range(5)
        ]
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        self.assertEqual(
            {row["app_version"] for row in report["cohorts"]},
            {"1.2.3", "unknown"},
        )
        self.assertEqual(report["alerts"], [])

    def test_untrusted_labels_are_collapsed_and_malformed_lines_are_counted(self) -> None:
        items = [
            event(
                "capture backend exceeded its active initialization budget",
                "2026-08-01T00:00:00Z",
                version="1.2.3",
                data={
                    "owner_kind": "dictation",
                    "backend": "private-device-name",
                    "last_setup_step": "unsafe\nstep",
                },
            ),
            event(
                "capture backend exceeded its active initialization budget",
                "2026-08-01T00:00:01Z",
                version="1.2.3",
                data={
                    "owner_kind": "dictation",
                    "backend": "cpal",
                    "last_setup_step": "none",
                },
            ),
        ]
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", items)
            path = Path(root) / "12345678-abcd" / "events.jsonl"
            with path.open("a", encoding="utf-8") as handle:
                handle.write("{broken\n")
            report = watch.build_report(root)

        self.assertEqual(report["malformed_lines"], 1)
        timeouts = report["cohorts"][0]["capture_backend_timeouts"]
        self.assertIn(
            {"backend": "unknown", "last_setup_step": "unknown", "count": 1},
            timeouts,
        )
        self.assertIn(
            {"backend": "cpal", "last_setup_step": "none", "count": 1},
            timeouts,
        )

    def test_transform_capture_events_do_not_enter_dictation_health_cohorts(self) -> None:
        events = self.ready_events("1.0.0", "01", [200] * 5)
        events += [
            event("startup_baseline", "2026-08-02T00:00:00Z", version="1.1.0"),
            event(
                "audio initialization accepted",
                "2026-08-02T00:00:01Z",
                version="1.1.0",
                data={"owner_kind": "transform"},
            ),
        ]
        events += [
            event(
                "audio readiness accepted",
                f"2026-08-02T00:00:{index + 2:02d}Z",
                version="1.1.0",
                data={
                    "event_code": "audio.capture_ready",
                    "owner_kind": "transform",
                    "startup_ms": 10_000,
                },
            )
            for index in range(5)
        ]
        events.append(
            event("startup_baseline", "2026-08-03T00:00:00Z", version="1.1.0")
        )
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        cohorts = {row["app_version"]: row for row in report["cohorts"]}
        self.assertEqual(cohorts["1.0.0"]["startup_sample_count"], 5)
        self.assertEqual(cohorts["1.1.0"]["startup_sample_count"], 0)
        self.assertEqual(cohorts["1.1.0"]["attempted_sessions"], 0)
        self.assertEqual(report["alerts"], [])

    def test_ready_recording_histogram_has_a_bounded_tail_bucket(self) -> None:
        version = "1.2.3"
        events = [
            event("startup_baseline", "2026-08-01T00:00:00Z", version=version),
            event(
                "audio initialization accepted",
                "2026-08-01T00:00:01Z",
                version=version,
                data={"owner_kind": "dictation"},
            ),
        ]
        events += [
            event(
                "audio readiness accepted",
                f"2026-08-01T00:{index // 60:02d}:{index % 60:02d}Z",
                version=version,
                data={
                    "event_code": "audio.capture_ready",
                    "owner_kind": "dictation",
                    "startup_ms": 100,
                },
            )
            for index in range(100)
        ]
        events.append(
            event("startup_baseline", "2026-08-02T00:00:00Z", version=version)
        )
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        self.assertEqual(
            report["cohorts"][0]["ready_recordings_per_session"],
            [
                {
                    "ready_recordings": watch.MAX_READY_RECORDINGS_PER_SESSION,
                    "ready_recordings_capped": True,
                    "sessions": 1,
                }
            ],
        )

    def test_stable_lifecycle_codes_flag_closed_missing_and_duplicate_terminals(self) -> None:
        version = "1.2.3"
        events = [
            event(
                "startup_baseline",
                "2026-08-01T00:00:00Z",
                version=version,
                data={"event_code": "system.startup_baseline"},
            ),
            event(
                "ignored",
                "2026-08-01T00:00:01Z",
                version=version,
                data={
                    "event_code": "audio.capture_started",
                    "recording_id": 1,
                    "owner_kind": "dictation",
                },
            ),
            event(
                "ignored",
                "2026-08-01T00:00:02Z",
                version=version,
                data={
                    "event_code": "audio.capture_started",
                    "recording_id": 2,
                    "owner_kind": "dictation",
                },
            ),
            event(
                "ignored",
                "2026-08-01T00:00:03Z",
                version=version,
                data={
                    "event_code": "pipeline.dictation_terminal",
                    "recording_id": 2,
                    "outcome": "success",
                },
            ),
            event(
                "ignored",
                "2026-08-01T00:00:04Z",
                version=version,
                data={
                    "event_code": "pipeline.dictation_terminal",
                    "recording_id": 2,
                    "outcome": "success",
                },
            ),
            event(
                "startup_baseline",
                "2026-08-02T00:00:00Z",
                version=version,
                data={"event_code": "system.startup_baseline"},
            ),
        ]
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        lifecycle_report = report["cohorts"][0]["dictation_lifecycle"]
        self.assertEqual(lifecycle_report["missing_terminals"], 1)
        self.assertEqual(lifecycle_report["duplicate_terminals"], 1)
        self.assertEqual(
            {alert["kind"] for alert in report["alerts"]},
            {"missing_dictation_terminals", "duplicate_dictation_terminals"},
        )

    def test_lifecycle_events_cannot_cross_the_startup_version_boundary(self) -> None:
        events = [
            event(
                "startup_baseline",
                "2026-08-01T00:00:00Z",
                version="1.0.0",
                data={"event_code": "system.startup_baseline"},
            ),
            event(
                "ignored",
                "2026-08-01T00:00:01Z",
                version="1.0.0",
                data={
                    "event_code": "audio.capture_started",
                    "recording_id": 7,
                    "owner_kind": "dictation",
                },
            ),
            event(
                "ignored",
                "2026-08-01T00:00:02Z",
                version="1.1.0",
                data={
                    "event_code": "pipeline.dictation_terminal",
                    "recording_id": 7,
                    "outcome": "success",
                },
            ),
            event(
                "startup_baseline",
                "2026-08-02T00:00:00Z",
                version="1.1.0",
                data={"event_code": "system.startup_baseline"},
            ),
        ]
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        cohorts = {row["app_version"]: row for row in report["cohorts"]}
        self.assertEqual(
            cohorts["1.0.0"]["dictation_lifecycle"]["missing_terminals"], 1
        )
        self.assertEqual(
            cohorts["1.1.0"]["dictation_lifecycle"]["orphan_stage_attempts"], 0
        )

    def test_version_cohort_cardinality_is_bounded(self) -> None:
        events = [
            event(
                "audio readiness accepted",
                f"2026-08-01T00:{index:02d}:00Z",
                version=f"1.0.{index}",
                data={"owner_kind": "dictation", "startup_ms": index},
            )
            for index in range(watch.MAX_VERSIONS_PER_INSTALL + 3)
        ]
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            report = watch.build_report(root)

        versions = {row["app_version"] for row in report["cohorts"]}
        self.assertEqual(len(versions), watch.MAX_VERSIONS_PER_INSTALL + 1)
        self.assertIn("overflow", versions)

    def test_cli_writes_report_before_returning_alert_exit(self) -> None:
        events = self.ready_events("1.0.0", "01", [100] * 5)
        events += self.ready_events("1.1.0", "02", [300] * 5)
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", events)
            output = Path(root) / "watch.json"
            result = watch.main(
                [
                    "--root",
                    root,
                    "--output",
                    str(output),
                    "--fail-on-alert",
                ]
            )
            saved = json.loads(output.read_text(encoding="utf-8"))

        self.assertEqual(result, 2)
        self.assertEqual(saved["schema_version"], 1)
        self.assertEqual(saved["status"], "alert")


if __name__ == "__main__":
    unittest.main()
