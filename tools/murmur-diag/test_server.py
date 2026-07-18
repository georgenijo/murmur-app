#!/usr/bin/env python3
"""Tests for murmur-diag log discovery and source isolation."""

import json
import tempfile
import unittest
from pathlib import Path

import server


class LogIngestionTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory()
        self.original_log_dir = server.LOG_DIR
        server.LOG_DIR = Path(self.temp_dir.name)

    def tearDown(self) -> None:
        server.LOG_DIR = self.original_log_dir
        self.temp_dir.cleanup()

    def write_events(self, filename: str, *events: dict) -> None:
        content = "".join(json.dumps(event) + "\n" for event in events)
        (server.LOG_DIR / filename).write_text(content, encoding="utf-8")

    def write_log(self, filename: str, content: str) -> None:
        (server.LOG_DIR / filename).write_text(content, encoding="utf-8")

    @staticmethod
    def event(timestamp: str, summary: str, stream: str = "system") -> dict:
        return {
            "timestamp": timestamp,
            "stream": stream,
            "level": "info",
            "summary": summary,
            "data": {},
        }

    def test_reads_release_and_dev_current_and_rotated_files_with_sources(self) -> None:
        fixtures = (
            ("events.jsonl.1", "release rotated"),
            ("events.jsonl", "release current"),
            ("events.dev.jsonl.1", "dev rotated"),
            ("events.dev.jsonl", "dev current"),
        )
        for index, (filename, summary) in enumerate(fixtures):
            self.write_events(
                filename,
                self.event(f"2026-07-18T12:00:0{index}Z", summary),
            )

        events = server.read_jsonl_files()

        self.assertEqual([event["summary"] for event in events], [item[1] for item in fixtures])
        self.assertEqual(
            [(event["diag_source"]["build"], event["diag_source"]["file"]) for event in events],
            [
                ("release", "events.jsonl.1"),
                ("release", "events.jsonl"),
                ("dev", "events.dev.jsonl.1"),
                ("dev", "events.dev.jsonl"),
            ],
        )

    def test_overlapping_patterns_do_not_ingest_a_file_twice(self) -> None:
        self.write_events(
            "events.jsonl",
            self.event("2026-07-18T12:00:00Z", "only once"),
        )

        events = server.read_jsonl_files("events*.jsonl", "events.jsonl")

        self.assertEqual(len(events), 1)
        self.assertEqual(events[0]["summary"], "only once")

    def test_keyboard_correlation_does_not_cross_build_sources(self) -> None:
        self.write_events(
            "events.jsonl",
            self.event(
                "2026-07-18T12:00:00.000Z",
                "BOTH -> timer promoted to hold-down-start",
                "keyboard",
            ),
        )
        self.write_events(
            "events.dev.jsonl",
            self.event(
                "2026-07-18T12:00:00.100Z",
                "start_native_recording",
                "pipeline",
            ),
        )

        result = server.correlate_keyboard(since=None)

        self.assertEqual(result["correlated"], 0)
        self.assertEqual(result["sources"]["release"]["missed"], 1)
        self.assertEqual(result["sources"]["dev"]["recording_starts"], 1)

    def test_search_logs_includes_release_and_dev_files(self) -> None:
        self.write_log("app.log", "release needle\n")
        self.write_log("app.dev.log", "dev needle\n")

        result = server.search_logs("needle")

        self.assertEqual(result["total_matched"], 2)
        self.assertEqual(result["sources"], ["dev", "release"])
        self.assertEqual(
            {(match["file"], match["source"]) for match in result["matches"]},
            {("app.log", "release"), ("app.dev.log", "dev")},
        )

    def test_search_logs_parses_tracing_format_and_applies_time_range(self) -> None:
        self.write_log(
            "app.dev.log",
            "2026-07-18T12:00:00.000000Z  INFO system: old needle\n"
            "2026-07-18T12:05:00.000000Z  WARN keyboard: recent needle\n",
        )

        result = server.search_logs(
            "needle",
            since="2026-07-18T12:04:00Z",
            until="2026-07-18T12:06:00Z",
        )

        self.assertEqual(result["total_matched"], 1)
        self.assertEqual(result["matches"][0]["timestamp"], "2026-07-18T12:05:00.000000Z")
        self.assertEqual(result["matches"][0]["level"], "WARN")
        self.assertEqual(result["matches"][0]["message"], "recent needle")


if __name__ == "__main__":
    unittest.main()
