from __future__ import annotations

import fcntl
import gzip
import importlib.util
import io
import json
from contextlib import redirect_stdout
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock


RECEIVER_DIRECTORY = Path(__file__).resolve().parents[1] / "infra" / "log-receiver"
if str(RECEIVER_DIRECTORY) not in sys.path:
    sys.path.insert(0, str(RECEIVER_DIRECTORY))

import event_store as store_module  # noqa: E402

SPEC = importlib.util.spec_from_file_location(
    "scrub_key_identities", RECEIVER_DIRECTORY / "scrub_key_identities.py"
)
assert SPEC and SPEC.loader
scrub = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(scrub)

INSTALL_ID = "93a0fc9c-0000-4000-8000-000000000001"
CLEAN_ID = "4b0b6f39-0000-4000-8000-000000000002"
IDENTITIES = ("KeyI", "KeyJ", "Num4", "Slash", "SemiColon", r'Some(\"q\")')


def keyboard(sequence: int, key: str, *, timestamp: str | None = None) -> dict:
    return {
        "timestamp": timestamp or "2026-10-%02dT00:00:%02dZ" % (1 + sequence // 60, sequence % 60),
        "stream": "keyboard",
        "level": "info",
        "summary": "keyboard event rejected",
        "data": {
            "reason": "not_target_key",
            "event_type": "KeyPress(%s)" % key,
            "key": "Some(%s)" % key,
            "target_key": "Some(ShiftLeft)",
            "hold_state": "Some(Idle)",
            "sequence": sequence,
        },
        "ingest_app_version": "0.52.2",
    }


def heartbeat(sequence: int) -> dict:
    return {
        "timestamp": "2026-10-20T00:00:%02dZ" % sequence,
        "stream": "system",
        "level": "info",
        "summary": "heartbeat",
        "data": {"sequence": sequence},
        "ingest_app_version": "0.53.0",
    }


def raw_text(directory: Path) -> str:
    payload = []
    for source in store_module.raw_sources(directory):
        with store_module.open_raw_source(source) as handle:
            payload.append(handle.read().decode("utf-8"))
    return "".join(payload)


class KeyScrubTests(unittest.TestCase):
    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.database = self.root / "events.sqlite3"
        self.store = store_module.EventStore(self.database)
        self.store.initialize()
        self.install = self.root / INSTALL_ID
        self.archive = self.install / "events.jsonl"

        # Seed exactly what the pre-sanitization receiver stored.
        with mock.patch.object(store_module, "sanitize_event", lambda event: (event, 0)):
            self.ingest(
                [keyboard(0, "KeyI"), keyboard(1, "Num4"), keyboard(2, "SemiColon")]
                + [heartbeat(sequence) for sequence in range(3)]
            )
            quota = self.archive.stat().st_size + 200
            self.ingest(
                [
                    # Same event except the typed key: merges once scrubbed.
                    keyboard(3, "KeyI", timestamp="2026-10-05T00:00:00Z"),
                    keyboard(3, "KeyJ", timestamp="2026-10-05T00:00:00Z"),
                    keyboard(4, "Slash"),
                ],
                quota=quota,
                seal=True,
            )
            self.assertTrue(store_module.compact_pending_segments(self.install))
            item = keyboard(5, "KeyB")
            item["data"]["event_name"] = 'Some("q")'
            self.ingest([item, heartbeat(9)])
            store_module.backfill(self.store, self.root, max_lines=1_000)
            report = store_module.reconcile(self.store, self.root)
        self.assertTrue(report["database_ready"])
        (self.install / "events.dev.jsonl").write_text(
            json.dumps(keyboard(7, "KeyZ")) + "\n", encoding="utf-8"
        )
        clean = self.root / CLEAN_ID
        clean.mkdir()
        self.store.ingest_batch(
            CLEAN_ID,
            [heartbeat(1)],
            metadata={},
            archive_path=str(clean / "events.jsonl"),
        )
        store_module.backfill(self.store, self.root, max_lines=1_000)
        self.store.set_dashboard_ready(True)

    def ingest(self, events, *, quota=None, seal=False):
        return self.store.ingest_batch(
            INSTALL_ID,
            events,
            metadata={"device_name": "Synthetic Mac"},
            archive_path=str(self.archive),
            archive_quota_bytes=quota,
            seal_full_archive=seal,
            received_at=1_790_000_000,
        )

    def snapshot(self) -> dict[str, bytes]:
        files = {}
        for path in sorted(self.root.rglob("*")):
            if path.is_file() and not path.name.startswith(
                ("events.sqlite3", scrub.MAINTENANCE_LOCK_NAME)
            ):
                files[str(path.relative_to(self.root))] = path.read_bytes()
        connection = self.store._connect(write=False)
        try:
            files["rows"] = json.dumps(
                [
                    tuple(row)
                    for row in connection.execute(
                        "SELECT event_id, event_hash, event_json FROM events ORDER BY event_id"
                    )
                ]
            ).encode()
        finally:
            connection.close()
        return files

    def run_tool(self, *arguments: str) -> tuple[int, dict | None]:
        output = io.StringIO()
        with redirect_stdout(output):
            code = scrub.main(["--root", str(self.root), *arguments])
        text = output.getvalue().strip()
        return code, json.loads(text) if text else None

    def database_text(self) -> str:
        connection = self.store._connect(write=False)
        try:
            return "\n".join(
                row[0] + row[1]
                for row in connection.execute("SELECT event_json, summary FROM events")
            )
        finally:
            connection.close()

    def test_dry_run_is_the_default_and_reports_counts_without_mutation(self) -> None:
        before = self.snapshot()

        code, report = self.run_tool()

        self.assertEqual(code, 0)
        self.assertEqual(report["mode"], "dry-run")
        self.assertEqual(self.snapshot(), before)
        by_install = {item["install_id"]: item for item in report["installs"]}
        affected = by_install[INSTALL_ID]
        self.assertEqual(affected["raw_sources"], 3)
        # 7 keyboard events (2 values each, plus one key name) and 1 dev line.
        self.assertEqual(affected["raw_lines_changed"], 8)
        self.assertEqual(affected["raw_values_redacted"], 17)
        self.assertEqual(affected["raw_sources_rewritten"], 0)
        self.assertEqual(affected["raw_lines_deduplicated"], 1)
        self.assertEqual(affected["database_rows_changed"], 7)
        self.assertEqual(affected["database_rows_merged"], 1)
        self.assertEqual(by_install[CLEAN_ID]["raw_lines_changed"], 0)
        self.assertEqual(by_install[CLEAN_ID]["database_rows_changed"], 0)
        for identity in IDENTITIES:
            self.assertIn(identity, raw_text(self.install) + self.database_text())

    def test_apply_scrubs_raw_segments_database_and_keeps_reconciliation(self) -> None:
        sealed_before = sorted(path.name for path in (self.install / "sealed").iterdir())

        code, report = self.run_tool("--apply")

        self.assertEqual(code, 0)
        affected = next(item for item in report["installs"] if item["install_id"] == INSTALL_ID)
        self.assertEqual(affected["raw_sources_rewritten"], 3)
        self.assertEqual(affected["database_rows_changed"], 7)
        self.assertEqual(affected["database_rows_merged"], 1)
        text = raw_text(self.install) + (self.install / "events.dev.jsonl").read_text()
        database = self.database_text()
        for identity in IDENTITIES + ("KeyZ", "KeyB"):
            self.assertNotIn(identity, text)
            self.assertNotIn(identity, database)
        self.assertIn("KeyPress(non_hotkey_key)", text)
        self.assertIn("Some(redacted_text)", database)
        self.assertIn("Some(ShiftLeft)", database)
        self.assertIn("Some(Idle)", text)
        # No unscrubbed backup or temporary file is left anywhere.
        self.assertEqual(
            sorted(path.name for path in (self.install / "sealed").iterdir()), sealed_before
        )
        leftovers = [
            path.name
            for path in self.root.rglob("*")
            if path.name.endswith(".tmp") or "scrub" in path.name
        ]
        self.assertEqual(leftovers, [])
        with gzip.open(next((self.install / "sealed").iterdir()), "rb") as handle:
            self.assertNotIn(b"KeyI", handle.read())

        reconciliation = store_module.reconcile(self.store, self.root)
        self.assertTrue(reconciliation["database_ready"], reconciliation)
        self.assertTrue(self.store.is_dashboard_ready())
        self.assertEqual(self.store.integrity_check(), "ok")
        page = self.store.query_events(
            store_module.EventQuery(install_id=INSTALL_ID, search="keyboard rejected")
        )
        self.assertEqual(len(page.events), 6)
        backfill = store_module.backfill(self.store, self.root, max_lines=1_000)
        self.assertTrue(backfill["complete"])
        self.assertEqual(backfill["inserted_events"], 0)

        code, again = self.run_tool("--apply")
        self.assertEqual(code, 0)
        self.assertEqual(again["totals"]["raw_lines_changed"], 0)
        self.assertEqual(again["totals"]["database_rows_changed"], 0)

    def test_retry_from_old_client_after_scrub_is_deduplicated(self) -> None:
        self.run_tool("--apply")
        count = self.store.event_count(INSTALL_ID)

        retry = self.ingest([keyboard(0, "KeyI"), keyboard(4, "Slash")])

        self.assertEqual((retry.inserted, retry.duplicates), (0, 2))
        self.assertEqual(self.store.event_count(INSTALL_ID), count)

    def test_embedded_summaries_and_typed_names_are_scrubbed_from_history_and_fts(self) -> None:
        legacy = heartbeat(30)
        legacy["summary"] = "rejected KeyPress(Num4) in listener"
        named = keyboard(31, "ShiftLeft")
        named["data"]["event_name"] = "a"
        with mock.patch.object(store_module, "sanitize_event", lambda event: (event, 0)):
            self.ingest([legacy, named])

        code, report = self.run_tool("--apply")

        self.assertEqual(code, 0)
        affected = next(item for item in report["installs"] if item["install_id"] == INSTALL_ID)
        self.assertEqual(affected["database_summaries_changed"], 1)
        text = raw_text(self.install)
        database = self.database_text()
        self.assertNotIn("Num4", text + database)
        self.assertNotIn('"event_name":"a"', text + database)
        self.assertIn("rejected KeyPress(non_hotkey_key) in listener", text)
        self.assertEqual(
            self.store.query_events(
                store_module.EventQuery(install_id=INSTALL_ID, search="Num4")
            ).events,
            (),
        )
        self.assertEqual(
            len(
                self.store.query_events(
                    store_module.EventQuery(install_id=INSTALL_ID, search="non_hotkey_key")
                ).events
            ),
            1,
        )
        store_module.backfill(self.store, self.root, max_lines=1_000)
        self.assertTrue(store_module.reconcile(self.store, self.root)["database_ready"])

    def test_post_deploy_retry_merges_into_the_earliest_row_and_first_raw_line(self) -> None:
        old = keyboard(40, "KeyQ", timestamp="2026-10-25T00:00:00Z")
        with mock.patch.object(store_module, "sanitize_event", lambda event: (event, 0)):
            self.ingest([old])
        connection = self.store._connect(write=False)
        try:
            original_id = connection.execute(
                "SELECT event_id FROM events WHERE event_json LIKE '%KeyQ%'"
            ).fetchone()[0]
        finally:
            connection.close()
        # The deployed receiver sanitizes the client's retry, so it hashes
        # differently from the stored unsanitized row and is inserted again.
        retry = self.ingest([old])
        self.assertEqual(retry.inserted, 1)

        code, report = self.run_tool("--apply")

        self.assertEqual(code, 0)
        affected = next(item for item in report["installs"] if item["install_id"] == INSTALL_ID)
        # The fixture's KeyI/KeyJ pair plus this old row's retry.
        self.assertEqual(affected["raw_lines_deduplicated"], 2)
        connection = self.store._connect(write=False)
        try:
            rows = connection.execute(
                "SELECT event_id FROM events WHERE event_json LIKE '%\"sequence\":40%'"
            ).fetchall()
        finally:
            connection.close()
        self.assertEqual([row[0] for row in rows], [original_id])
        lines = [line for line in raw_text(self.install).splitlines() if '"sequence":40' in line]
        self.assertEqual(len(lines), 1)
        store_module.backfill(self.store, self.root, max_lines=1_000)
        self.assertTrue(store_module.reconcile(self.store, self.root)["database_ready"])
        self.assertTrue(store_module.backfill(self.store, self.root, max_lines=1_000)["complete"])

    def test_interrupted_scrub_still_removes_a_cross_segment_retry(self) -> None:
        old = keyboard(50, "KeyW", timestamp="2026-10-26T00:00:00Z")
        with mock.patch.object(store_module, "sanitize_event", lambda event: (event, 0)):
            self.ingest([old])
        # Seal the old event into segment 2; its sanitized retry lands live.
        store_module.seal_live_file(self.install)
        store_module.compact_pending_segments(self.install)
        self.assertEqual(self.ingest([old]).inserted, 1)
        original = scrub._apply_checkpoint

        def crash_after_sealed_rewrite(store, entry):
            if entry["key"].endswith("#2"):
                raise OSError("synthetic crash after rename")
            return original(store, entry)

        with mock.patch.object(scrub, "_apply_checkpoint", crash_after_sealed_rewrite):
            with self.assertRaises(OSError):
                self.run_tool("--apply")
        code, report = self.run_tool("--apply")

        self.assertEqual(code, 0)
        affected = next(item for item in report["installs"] if item["install_id"] == INSTALL_ID)
        self.assertEqual(affected["raw_lines_deduplicated"], 1)
        lines = [line for line in raw_text(self.install).splitlines() if '"sequence":50' in line]
        self.assertEqual(len(lines), 1)
        connection = self.store._connect(write=False)
        try:
            rows = connection.execute(
                "SELECT COUNT(*) FROM events WHERE event_json LIKE '%\"sequence\":50%'"
            ).fetchone()[0]
        finally:
            connection.close()
        self.assertEqual(rows, 1)
        store_module.backfill(self.store, self.root, max_lines=1_000)
        self.assertTrue(store_module.reconcile(self.store, self.root)["database_ready"])

    def test_apply_refuses_while_the_receiver_holds_the_maintenance_lock(self) -> None:
        before = self.snapshot()
        with open(self.root / scrub.MAINTENANCE_LOCK_NAME, "a+b") as lock:
            fcntl.flock(lock.fileno(), fcntl.LOCK_SH)
            code, report = self.run_tool("--apply")

        self.assertEqual((code, report), (2, None))
        self.assertEqual(self.snapshot(), before)

    def test_interrupted_rewrite_is_finished_by_the_next_run(self) -> None:
        original = scrub._apply_checkpoint
        calls = []

        def crash_once(store, entry):
            calls.append(entry["key"])
            if len(calls) == 1:
                raise OSError("synthetic crash after rename")
            return original(store, entry)

        with mock.patch.object(scrub, "_apply_checkpoint", crash_once):
            with self.assertRaises(OSError):
                self.run_tool("--apply")
            self.assertTrue((self.root / scrub.JOURNAL_NAME).exists())
            code, _ = self.run_tool("--apply")

        self.assertEqual(code, 0)
        self.assertFalse((self.root / scrub.JOURNAL_NAME).exists())
        self.assertTrue(store_module.reconcile(self.store, self.root)["database_ready"])
        self.assertTrue(store_module.backfill(self.store, self.root, max_lines=1_000)["complete"])


if __name__ == "__main__":
    unittest.main()
