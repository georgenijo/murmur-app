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
                data={"event_code": "audio.capture_ready", "startup_ms": value},
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
                        "backend": "auhal",
                        "last_setup_step": "stream_start",
                    },
                ),
                event(
                    "capture backend failed before retained audio; trying bounded fallback",
                    "2026-08-01T00:01:01Z",
                    version="1.2.3",
                    data={"event_code": "audio.fallback_started"},
                ),
                event(
                    "both capture backend attempts failed before first PCM",
                    "2026-08-01T00:01:02Z",
                    version="1.2.3",
                    data={"event_code": "audio.capture_failed"},
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
            [{"ready_recordings": 0, "sessions": 2}],
        )
        self.assertEqual(report["alerts"][0]["kind"], "repeated_zero_ready_sessions")

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
                data={"startup_ms": 100},
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
        item = event(
            "capture backend exceeded its active initialization budget",
            "2026-08-01T00:00:00Z",
            version="1.2.3",
            data={
                "backend": "private-device-name",
                "last_setup_step": "unsafe\nstep",
            },
        )
        with tempfile.TemporaryDirectory() as root:
            self.write_install(root, "12345678-abcd", [item])
            path = Path(root) / "12345678-abcd" / "events.jsonl"
            with path.open("a", encoding="utf-8") as handle:
                handle.write("{broken\n")
            report = watch.build_report(root)

        self.assertEqual(report["malformed_lines"], 1)
        timeout = report["cohorts"][0]["capture_backend_timeouts"][0]
        self.assertEqual(timeout["backend"], "unknown")
        self.assertEqual(timeout["last_setup_step"], "unknown")

    def test_version_cohort_cardinality_is_bounded(self) -> None:
        events = [
            event(
                "audio readiness accepted",
                f"2026-08-01T00:{index:02d}:00Z",
                version=f"1.0.{index}",
                data={"startup_ms": index},
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
