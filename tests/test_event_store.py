from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock


STORE_PATH = (
    Path(__file__).resolve().parents[1]
    / "infra"
    / "log-receiver"
    / "event_store.py"
)
SPEC = importlib.util.spec_from_file_location("murmur_event_store", STORE_PATH)
assert SPEC and SPEC.loader
store_module = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = store_module
SPEC.loader.exec_module(store_module)


def event(
    summary: str,
    *,
    timestamp: str = "2026-08-10T12:00:00Z",
    level: str = "info",
    stream: str = "system",
    version: str = "1.2.3",
    sequence: int = 1,
) -> dict:
    return {
        "timestamp": timestamp,
        "stream": stream,
        "level": level,
        "summary": summary,
        "data": {"sequence": sequence},
        "ingest_app_version": version,
    }


class EventStoreTestCase(unittest.TestCase):
    install_id = "12345678-abcd"

    def setUp(self) -> None:
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.database = self.root / "events.sqlite3"
        self.archive = self.root / self.install_id / "events.jsonl"
        self.store = store_module.EventStore(self.database)
        self.store.initialize()

    def ingest(self, events: list[dict]) -> object:
        return self.store.ingest_batch(
            self.install_id,
            events,
            metadata={"device_name": "Test Mac", "last_version": "1.2.3"},
            archive_path=str(self.archive),
            received_at=1_786_000_000,
        )


class EventStoreMigrationTests(EventStoreTestCase):
    def test_migrations_reopen_with_wal_foreign_keys_and_integrity(self) -> None:
        cursor_secret = self.store.cursor_secret()
        reopened = store_module.EventStore(self.database)
        reopened.initialize()

        connection = reopened._connect()
        try:
            self.assertEqual(connection.execute("PRAGMA foreign_keys").fetchone()[0], 1)
            self.assertEqual(
                connection.execute("PRAGMA busy_timeout").fetchone()[0],
                store_module.DEFAULT_BUSY_TIMEOUT_MS,
            )
            self.assertEqual(
                connection.execute("PRAGMA journal_mode").fetchone()[0].lower(),
                "wal",
            )
            self.assertEqual(connection.execute("PRAGMA synchronous").fetchone()[0], 2)
            self.assertEqual(
                connection.execute("PRAGMA journal_size_limit").fetchone()[0],
                16 * 1024 * 1024,
            )
        finally:
            connection.close()
        self.assertEqual(reopened.schema_version(), store_module.SCHEMA_VERSION)
        self.assertEqual(reopened.integrity_check(), "ok")
        self.assertEqual(reopened.cursor_secret(), cursor_secret)
        self.assertEqual(len(cursor_secret), 64)

    def test_concurrent_initializers_apply_migration_idempotently(self) -> None:
        database = self.root / "concurrent.sqlite3"
        barrier = threading.Barrier(5)
        errors: list[BaseException] = []

        def initialize() -> None:
            try:
                barrier.wait()
                store_module.EventStore(database).initialize()
            except BaseException as error:
                errors.append(error)

        threads = [threading.Thread(target=initialize) for _ in range(4)]
        for thread in threads:
            thread.start()
        barrier.wait()
        for thread in threads:
            thread.join(5)

        self.assertEqual(errors, [])
        self.assertEqual(store_module.EventStore(database).schema_version(), 1)

    def test_corrupt_database_fails_closed(self) -> None:
        corrupt = self.root / "corrupt.sqlite3"
        corrupt.write_bytes(b"not a sqlite database")

        with self.assertRaises(store_module.StoreCorrupt):
            store_module.EventStore(corrupt).initialize()


class EventStoreIngestTests(EventStoreTestCase):
    def test_batch_is_transactional_and_exact_retry_is_idempotent(self) -> None:
        events = [event("one", sequence=1), event("two", sequence=2)]

        first = self.ingest(events)
        retry = self.ingest(events)

        self.assertEqual((first.inserted, first.duplicates), (2, 0))
        self.assertEqual((retry.inserted, retry.duplicates), (0, 2))
        self.assertEqual(self.store.event_count(self.install_id), 2)
        raw = [json.loads(line) for line in self.archive.read_text().splitlines()]
        self.assertEqual([item["summary"] for item in raw], ["one", "two"])

    def test_complete_validation_happens_before_database_or_archive_mutation(self) -> None:
        with self.assertRaises(store_module.InvalidEvent):
            self.ingest([event("valid"), {"bad": float("nan")}])

        self.assertFalse(self.archive.exists())
        self.assertEqual(self.store.list_installs(), [])

    def test_commit_failure_rolls_database_and_archive_back(self) -> None:
        with mock.patch.object(
            self.store,
            "_commit",
            side_effect=store_module.StoreCommitError("synthetic commit failure"),
        ):
            with self.assertRaises(store_module.StoreCommitError):
                self.ingest([event("not committed")])

        self.assertEqual(self.store.event_count(self.install_id), 0)
        self.assertFalse(self.archive.exists())

    def test_archive_failure_rolls_database_back(self) -> None:
        with mock.patch.object(
            self.store,
            "_append_archive",
            side_effect=store_module.ArchiveError("synthetic disk full"),
        ):
            with self.assertRaises(store_module.ArchiveError):
                self.ingest([event("not archived")])

        self.assertEqual(self.store.event_count(self.install_id), 0)

    def test_partial_archive_fsync_failure_restores_file_and_new_directory(self) -> None:
        with mock.patch.object(
            store_module.os,
            "fsync",
            side_effect=[OSError("synthetic disk full"), None],
        ), self.assertRaises(store_module.ArchiveError):
            self.ingest([event("partially written")])

        self.assertEqual(self.store.list_installs(), [])
        self.assertFalse(self.archive.exists())
        self.assertFalse(self.archive.parent.exists())

    def test_sqlite_disk_full_is_classified_as_quota_and_not_archived(self) -> None:
        with mock.patch.object(
            self.store,
            "_insert_prepared",
            side_effect=sqlite3.OperationalError("database or disk is full"),
        ):
            with self.assertRaises(store_module.StoreQuota):
                self.ingest([event("full")])

        self.assertFalse(self.archive.exists())

    def test_database_quota_fails_without_partial_archive(self) -> None:
        limited = store_module.EventStore(self.root / "limited.sqlite3", quota_bytes=1024 * 1024)
        limited.initialize()
        large_events = []
        for sequence in range(40):
            item = event("quota", sequence=sequence)
            item["data"]["padding"] = ("x" * 40_000) + str(sequence)
            large_events.append(item)

        with self.assertRaises(store_module.StoreQuota):
            limited.ingest_batch(
                self.install_id,
                large_events,
                metadata={},
                archive_path=str(self.root / "quota" / "events.jsonl"),
            )

        self.assertFalse((self.root / "quota" / "events.jsonl").exists())

    def test_request_scoped_connections_wait_only_for_bounded_busy_timeout(self) -> None:
        blocker = self.store._connect()
        blocker.execute("BEGIN IMMEDIATE")
        contender = store_module.EventStore(self.database, busy_timeout_ms=100)
        started = time.monotonic()
        try:
            with self.assertRaises(store_module.StoreBusy):
                contender.ingest_batch(
                    self.install_id,
                    [event("busy")],
                    metadata={},
                    archive_path=str(self.root / "busy" / "events.jsonl"),
                )
        finally:
            blocker.execute("ROLLBACK")
            blocker.close()
        elapsed = time.monotonic() - started
        self.assertGreaterEqual(elapsed, 0.08)
        self.assertLess(elapsed, 2.0)

    def test_concurrent_request_scoped_connections_commit_complete_batches(self) -> None:
        barrier = threading.Barrier(5)
        errors: list[BaseException] = []

        def write(sequence: int) -> None:
            try:
                barrier.wait()
                local = store_module.EventStore(self.database)
                local.ingest_batch(
                    self.install_id,
                    [event("parallel-%d" % sequence, sequence=sequence)],
                    metadata={},
                    archive_path=str(self.archive),
                )
            except BaseException as error:
                errors.append(error)

        threads = [threading.Thread(target=write, args=(sequence,)) for sequence in range(4)]
        for thread in threads:
            thread.start()
        barrier.wait()
        for thread in threads:
            thread.join(5)

        self.assertEqual(errors, [])
        self.assertEqual(self.store.event_count(self.install_id), 4)
        self.assertEqual(len(self.archive.read_text().splitlines()), 4)


class EventStoreBackfillTests(EventStoreTestCase):
    def test_bounded_resume_malformed_non_object_duplicates_and_reconciliation(self) -> None:
        self.archive.parent.mkdir(parents=True)
        valid = event("historical", timestamp="2026-08-01T00:00:00Z")
        self.archive.write_text(
            "\n".join(
                (
                    json.dumps(valid),
                    "not-json",
                    json.dumps(["not", "an", "object"]),
                    json.dumps(valid),
                )
            )
            + "\n",
            encoding="utf-8",
        )
        (self.archive.parent / "meta.json").write_text(
            json.dumps({"device_name": "Backfill Mac", "last_version": "1.0.0"}),
            encoding="utf-8",
        )

        first = store_module.backfill(self.store, self.root, max_lines=2, batch_size=2)
        second = store_module.backfill(self.store, self.root, max_lines=2, batch_size=2)
        report = store_module.reconcile(self.store, self.root, mark_ready=True)

        self.assertFalse(first["complete"])
        self.assertTrue(second["complete"])
        self.assertEqual(self.store.event_count(self.install_id), 1)
        install = report["installs"][0]
        self.assertEqual(install["raw_lines"], 4)
        self.assertEqual(install["valid_objects"], 2)
        self.assertEqual(install["malformed_lines"], 2)
        self.assertEqual(install["duplicates"], 1)
        self.assertEqual(install["untimed_events"], 0)
        self.assertEqual(install["inserted_events"], 1)
        self.assertEqual(install["database_count"], 1)
        self.assertEqual(install["earliest_timestamp"], "2026-08-01T00:00:00Z")
        self.assertEqual(install["latest_timestamp"], "2026-08-01T00:00:00Z")
        self.assertTrue(report["database_ready"])
        self.assertTrue(self.store.is_dashboard_ready())

    def test_reconciliation_cannot_enable_dashboard_when_counts_differ(self) -> None:
        self.archive.parent.mkdir(parents=True)
        self.archive.write_text(json.dumps(event("missing")) + "\n", encoding="utf-8")

        with self.assertRaises(store_module.StoreError):
            store_module.reconcile(self.store, self.root, mark_ready=True)

        self.assertFalse(self.store.is_dashboard_ready())


class EventStoreQueryTests(EventStoreTestCase):
    def setUp(self) -> None:
        super().setUp()
        self.ingest(
            [
                event(
                    "capture warning old",
                    timestamp="2026-08-01T12:00:00Z",
                    level="warn",
                    stream="audio",
                    version="1.0.0",
                    sequence=1,
                ),
                event(
                    "capture recovered",
                    timestamp="2026-08-02T12:00:00Z",
                    level="info",
                    stream="audio",
                    version="1.1.0",
                    sequence=2,
                ),
                event(
                    "update failed",
                    timestamp="2026-08-03T12:00:00Z",
                    level="error",
                    stream="updater",
                    version="1.1.0",
                    sequence=3,
                ),
                event(
                    "normal system heartbeat",
                    timestamp="2026-08-04T12:00:00Z",
                    level="info",
                    stream="system",
                    version="1.2.0",
                    sequence=4,
                ),
            ]
        )

    def summaries(self, query: object) -> list[str]:
        return [item["summary"] for item in self.store.query_events(query).events]

    def test_every_filter_and_full_history_problem_view(self) -> None:
        start = store_module.timestamp_us("2026-08-02T00:00:00Z")
        end = store_module.timestamp_us("2026-08-03T23:59:59Z")
        cases = (
            (store_module.EventQuery(install_id=self.install_id), 4),
            (store_module.EventQuery(start_us=start, end_us=end), 2),
            (store_module.EventQuery(app_version="1.1.0"), 2),
            (store_module.EventQuery(level="error"), 1),
            (store_module.EventQuery(stream="audio"), 2),
            (store_module.EventQuery(search="capture recovered"), 1),
            (store_module.EventQuery(problems_only=True), 2),
        )
        for query, expected in cases:
            with self.subTest(query=query):
                self.assertEqual(len(self.store.query_events(query).events), expected)

    def test_untimed_warning_remains_visible_in_full_history(self) -> None:
        item = event("untimed warning", level="warn", sequence=5)
        del item["timestamp"]
        self.ingest([item])

        summaries = self.summaries(store_module.EventQuery(problems_only=True))

        self.assertIn("untimed warning", summaries)

    def test_stable_keyset_pages_with_identical_timestamps(self) -> None:
        same_time = "2026-08-10T00:00:00Z"
        self.ingest(
            [event("tie-%d" % sequence, timestamp=same_time, sequence=sequence) for sequence in range(10, 15)]
        )
        query = store_module.EventQuery(
            install_id=self.install_id,
            start_us=store_module.timestamp_us(same_time),
            limit=2,
        )
        seen = []
        before = None
        while True:
            page = self.store.query_events(query, before=before)
            seen.extend(item["summary"] for item in page.events)
            if page.next_position is None:
                break
            before = page.next_position

        self.assertEqual(seen, ["tie-14", "tie-13", "tie-12", "tie-11", "tie-10"])
        self.assertEqual(len(seen), len(set(seen)))

    def test_cursor_is_signed_bounded_and_bound_to_filters(self) -> None:
        query = store_module.EventQuery(install_id=self.install_id, level="error")
        position = (1_786_000_000_000_000, 42)
        cursor = store_module.encode_cursor("secret", query, position)
        self.assertEqual(store_module.decode_cursor("secret", query, cursor), position)

        with self.assertRaises(store_module.InvalidQuery):
            store_module.decode_cursor("secret", query, cursor[:-1] + "A")
        with self.assertRaises(store_module.InvalidQuery):
            store_module.decode_cursor(
                "secret", store_module.EventQuery(install_id=self.install_id), cursor
            )
        with self.assertRaises(store_module.InvalidQuery):
            store_module.decode_cursor("secret", query, "x" * 1025)

    def test_query_result_byte_budget_pages_without_skipping(self) -> None:
        same_time = "2026-08-11T00:00:00Z"
        padded = []
        for sequence in range(20, 23):
            item = event("bounded-%d" % sequence, timestamp=same_time, sequence=sequence)
            item["data"]["padding"] = "x" * 300
            padded.append(item)
        self.ingest(padded)
        query = store_module.EventQuery(
            install_id=self.install_id,
            start_us=store_module.timestamp_us(same_time),
            limit=10,
        )

        seen = []
        before = None
        with mock.patch.object(store_module, "MAX_QUERY_RESULT_BYTES", 700):
            while True:
                page = self.store.query_events(query, before=before)
                seen.extend(item["summary"] for item in page.events)
                if page.next_position is None:
                    break
                before = page.next_position
        self.assertEqual(seen, ["bounded-22", "bounded-21", "bounded-20"])

    def test_utc_and_eastern_date_parsing(self) -> None:
        utc = store_module.parse_local_datetime("2026-08-01T12:00", "utc")
        eastern = store_module.parse_local_datetime("2026-08-01T08:00", "eastern")
        self.assertEqual(utc, eastern)


class EventStoreOperationsTests(EventStoreTestCase):
    def test_backup_integrity_and_restore_preserve_a_pre_restore_copy(self) -> None:
        self.ingest([event("before backup")])
        backup = self.root / "backup?#%.sqlite3"
        self.store.backup(str(backup))
        self.ingest([event("after backup", sequence=2)])

        preserved = store_module.restore_database(str(self.database), str(backup))
        restored = store_module.EventStore(self.database)
        restored.initialize()

        self.assertTrue(Path(preserved).exists())
        self.assertEqual(restored.integrity_check(), "ok")
        self.assertEqual(restored.event_count(self.install_id), 1)

    def test_restore_recovers_when_current_database_is_corrupt(self) -> None:
        self.ingest([event("good backup")])
        backup = self.root / "backup.sqlite3"
        self.store.backup(str(backup))
        self.database.write_bytes(b"not a sqlite database")
        Path(str(self.database) + "-wal").write_bytes(b"wal evidence")
        Path(str(self.database) + "-shm").write_bytes(b"shm evidence")

        preserved = store_module.restore_database(str(self.database), str(backup))
        restored = store_module.EventStore(self.database)
        restored.initialize()

        self.assertEqual(Path(preserved).read_bytes(), b"not a sqlite database")
        self.assertEqual(Path(preserved + "-wal").read_bytes(), b"wal evidence")
        self.assertEqual(Path(preserved + "-shm").read_bytes(), b"shm evidence")
        self.assertEqual(restored.integrity_check(), "ok")
        self.assertEqual(restored.event_count(self.install_id), 1)

    def test_state_contract_rejects_legacy_and_accepts_current_aggregate(self) -> None:
        with self.assertRaises(store_module.StoreError):
            store_module.normalize_state_snapshot(
                {"default_input": "Private Mic", "input_devices": ["Private Mic"]}
            )
        normalized = store_module.normalize_state_snapshot(
            {
                "default_input_available": True,
                "input_device_count": 2,
                "input_device_count_capped": False,
                "input_enumeration_ok": True,
            },
            received_at=1_786_000_000,
        )
        self.store.update_state(self.install_id, normalized)

        install = self.store.get_install(self.install_id)
        assert install is not None
        self.assertEqual(install["state"]["input_device_count"], 2)
        self.assertNotIn("default_input", install["state"])


if __name__ == "__main__":
    unittest.main()
