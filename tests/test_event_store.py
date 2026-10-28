from __future__ import annotations

import importlib.util
import io
import json
from pathlib import Path
import sqlite3
import sys
import tempfile
import threading
import time
import unittest
from contextlib import redirect_stdout
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
        checkpoint = self.store.checkpoint_offset(
            f"{self.install_id}/events.jsonl"
        )
        with self.assertRaises(store_module.StoreError):
            self.store.import_backfill_chunk(
                self.install_id,
                f"{self.install_id}/events.jsonl",
                events=[valid],
                raw_lines=1,
                malformed_lines=0,
                start_offset=0,
                end_offset=checkpoint,
                source_size=self.archive.stat().st_size,
                source_mtime_ns=self.archive.stat().st_mtime_ns,
                complete=False,
            )
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


def raw_stream(directory: Path) -> bytes:
    """Concatenate every raw source the way recovery/export reads it."""
    payload = b""
    for source in store_module.raw_sources(directory):
        with store_module.open_raw_source(source) as handle:
            payload += handle.read()
    return payload


class EventStoreRawSealTests(EventStoreTestCase):
    def ingest_capped(self, events: list[dict], quota: int) -> object:
        return self.store.ingest_batch(
            self.install_id,
            events,
            metadata={"device_name": "Test Mac"},
            archive_path=str(self.archive),
            archive_quota_bytes=quota,
            seal_full_archive=True,
            received_at=1_786_000_000,
        )

    def batch(self, prefix: str, count: int, *, day: int = 1) -> list[dict]:
        return [
            event(
                "%s-%d" % (prefix, sequence),
                timestamp="2026-08-%02dT00:00:%02dZ" % (day, sequence),
                sequence=sequence,
            )
            for sequence in range(count)
        ]

    def test_cap_seals_live_file_into_verified_gzip_without_losing_history(self) -> None:
        first = self.batch("first", 4, day=1)
        self.ingest_capped(first, quota=10_000)
        before = self.archive.read_bytes()
        quota = len(before) + 50

        result = self.ingest_capped(self.batch("second", 3, day=2), quota=quota)
        pending = self.archive.parent / "sealed" / "events-000001.pending.jsonl"

        self.assertTrue(result.sealed)
        self.assertEqual(result.inserted, 3)
        self.assertEqual(pending.read_bytes(), before)
        self.assertEqual(
            [json.loads(line)["summary"] for line in self.archive.read_text().splitlines()],
            ["second-0", "second-1", "second-2"],
        )
        live_bytes = self.archive.read_bytes()

        self.assertEqual(store_module.compact_pending_segments(self.archive.parent), 1)
        sealed = sorted((self.archive.parent / "sealed").iterdir())
        self.assertEqual(
            [path.name for path in sealed],
            ["events-000001-20260801T000000Z-20260801T000003Z.jsonl.gz"],
        )
        sources = store_module.raw_sources(self.archive.parent)
        self.assertEqual(
            [(source.key, source.compressed, source.sealed) for source in sources],
            [
                (f"{self.install_id}/events.jsonl", True, True),
                (f"{self.install_id}/events.jsonl#2", False, False),
            ],
        )
        self.assertEqual(store_module.raw_source_size(sources[0]), len(before))
        self.assertEqual(raw_stream(self.archive.parent), before + live_bytes)
        self.assertEqual(self.store.event_count(self.install_id), 7)

    def test_exact_retry_after_seal_is_deduplicated_by_the_database(self) -> None:
        first = self.batch("first", 4)
        self.ingest_capped(first, quota=10_000)
        quota = self.archive.stat().st_size + 50
        second = self.batch("second", 3, day=2)
        self.ingest_capped(second, quota=quota)
        store_module.compact_pending_segments(self.archive.parent)
        stream = raw_stream(self.archive.parent)

        old_retry = self.ingest_capped(first, quota=quota)
        new_retry = self.ingest_capped(second, quota=quota)

        self.assertEqual((old_retry.inserted, old_retry.sealed), (0, False))
        self.assertEqual((new_retry.inserted, new_retry.sealed), (0, False))
        self.assertEqual(raw_stream(self.archive.parent), stream)
        self.assertEqual(self.store.event_count(self.install_id), 7)

    def test_failed_commit_after_seal_keeps_seal_and_removes_only_new_lines(self) -> None:
        self.ingest_capped(self.batch("first", 4), quota=10_000)
        before = self.archive.read_bytes()
        with mock.patch.object(
            self.store,
            "_commit",
            side_effect=store_module.StoreCommitError("synthetic commit failure"),
        ), self.assertRaises(store_module.StoreCommitError):
            self.ingest_capped(self.batch("second", 3, day=2), quota=len(before) + 50)

        self.assertFalse(self.archive.exists())
        self.assertEqual(raw_stream(self.archive.parent), before)
        self.assertEqual(self.store.event_count(self.install_id), 4)
        retry = self.ingest_capped(self.batch("second", 3, day=2), quota=len(before) + 50)
        self.assertEqual((retry.inserted, retry.sealed), (3, False))
        self.assertEqual(
            [source.key for source in store_module.raw_sources(self.archive.parent)],
            [f"{self.install_id}/events.jsonl", f"{self.install_id}/events.jsonl#2"],
        )

    def test_every_interrupted_compaction_state_recovers_to_one_segment(self) -> None:
        self.ingest_capped(self.batch("first", 4), quota=10_000)
        before = self.archive.read_bytes()
        sealed = self.archive.parent / "sealed"
        store_module.seal_live_file(self.archive.parent)
        pending = sealed / "events-000001.pending.jsonl"

        # Crash after the rename: the pending file alone is the segment.
        self.assertEqual(raw_stream(self.archive.parent), before)

        # Crash while writing the gzip temporary: it is never a source.
        (sealed / ".events-000001.dead.tmp").write_bytes(b"partial gzip")
        self.assertEqual(raw_stream(self.archive.parent), before)

        # Crash after publishing, before removing the pending file.
        store_module.compact_pending_segments(self.archive.parent)
        pending.write_bytes(before)
        self.assertEqual(raw_stream(self.archive.parent), before)
        self.assertEqual(store_module.compact_pending_segments(self.archive.parent), 1)
        self.assertEqual(len(list(sealed.iterdir())), 1)
        self.assertEqual(raw_stream(self.archive.parent), before)

        # A pending file that disagrees with its published gzip fails closed.
        pending.write_bytes(before + b"{}\n")
        with self.assertRaises(store_module.StoreCorrupt):
            store_module.compact_pending_segments(self.archive.parent)
        self.assertTrue(pending.exists())

    def test_sequence_gap_fails_closed(self) -> None:
        sealed = self.archive.parent / "sealed"
        sealed.mkdir(parents=True)
        (sealed / "events-000002.pending.jsonl").write_bytes(b"{}\n")

        with self.assertRaises(store_module.StoreCorrupt):
            store_module.raw_sources(self.archive.parent)

    def test_backfill_checkpoint_follows_the_stream_across_a_seal(self) -> None:
        self.ingest_capped(self.batch("first", 4), quota=10_000)
        # Backfill the live file into a second projection, then seal it.
        projection = store_module.EventStore(self.root / "projection.sqlite3")
        projection.initialize()
        store_module.backfill(projection, self.root, max_lines=100)
        self.ingest_capped(self.batch("second", 3, day=2), quota=self.archive.stat().st_size + 50)
        store_module.compact_pending_segments(self.archive.parent)
        self.ingest_capped(self.batch("third", 2, day=3), quota=10_000)

        report = store_module.backfill(projection, self.root, max_lines=100)
        reconciliation = store_module.reconcile(projection, self.root, mark_ready=True)

        self.assertTrue(report["complete"])
        self.assertEqual(report["inserted_events"], 5)
        install = reconciliation["installs"][0]
        self.assertTrue(install["ready"])
        self.assertEqual(install["raw_sources"], 2)
        self.assertEqual(install["database_count"], 9)
        self.assertTrue(store_module.reconcile(self.store, self.root)["installs"][0]["raw_lines"] == 9)

    def test_large_gzip_backfill_uses_bounded_batches_without_rewinding(self) -> None:
        events = []
        for sequence in range(12):
            item = event("large-%d" % sequence, sequence=sequence)
            item["data"]["padding"] = "x" * 600_000
            events.append(item)
        self.ingest_capped(events[:6], quota=100 * 1024 * 1024)
        self.ingest_capped(events[6:], quota=self.archive.stat().st_size + 1_000_000)
        store_module.compact_pending_segments(self.archive.parent)
        projection = store_module.EventStore(self.root / "projection.sqlite3")
        projection.initialize()
        original_seek = store_module.gzip.GzipFile.seek
        rewinds = []

        def forward_only(handle, offset, whence=0):
            # tell() is seek(0, SEEK_CUR); any move backwards rewinds.
            if (whence == 1 and offset < 0) or (
                whence == 0 and offset < original_seek(handle, 0, 1)
            ):
                rewinds.append(offset)
            return original_seek(handle, offset, whence)

        with mock.patch.object(
            store_module.gzip.GzipFile, "seek", forward_only
        ), mock.patch.object(store_module, "MAX_BACKFILL_BATCH_BYTES", 1_300_000):
            report = store_module.backfill(projection, self.root, max_lines=100)

        self.assertEqual(rewinds, [])
        self.assertTrue(report["complete"])
        self.assertTrue(store_module.reconcile(projection, self.root)["database_ready"])


    def test_pinned_open_survives_compaction_between_resolve_and_open(self) -> None:
        payload = b"".join(
            (json.dumps(event("bulk-%d" % sequence, sequence=sequence)) + "\n").encode()
            for sequence in range(200)
        )
        self.archive.parent.mkdir(parents=True)
        self.archive.write_bytes(payload)
        store_module.seal_live_file(self.archive.parent)
        original = store_module._current_source
        calls = []

        def compact_after_first_resolve(source):
            resolved = original(source)
            if not calls:
                calls.append(source)
                # Compaction finishes after resolution but before the open.
                store_module.compact_pending_segments(self.archive.parent)
            return resolved

        with mock.patch.object(store_module, "_current_source", compact_after_first_resolve):
            snapshot = store_module.open_raw_snapshot(self.archive.parent)
        try:
            (source, handle, size), = snapshot
            self.assertTrue(source.compressed)
            self.assertEqual(size, len(payload))
            self.assertEqual(handle.read(), payload)
        finally:
            store_module.close_snapshot(snapshot)
        self.assertEqual(calls and len(calls), 1)

    def test_seal_racing_backfill_open_cannot_skip_or_mislabel_events(self) -> None:
        self.ingest_capped(self.batch("first", 20), quota=100_000)
        projection = store_module.EventStore(self.root / "projection.sqlite3")
        projection.initialize()
        original = store_module.open_pinned_source
        raced = []

        def seal_before_live_open(source):
            if not source.sealed and not raced:
                raced.append(source)
                # An ingest seals the live file and appends one new event
                # after backfill listed the sources but before it opened them.
                self.ingest_capped(
                    self.batch("racing", 1, day=9),
                    quota=self.archive.stat().st_size + 1,
                )
            return original(source)

        with mock.patch.object(store_module, "open_pinned_source", seal_before_live_open):
            first = store_module.backfill(projection, self.root, max_lines=1_000)
        final = store_module.backfill(projection, self.root, max_lines=1_000)
        reconciliation = store_module.reconcile(projection, self.root)

        self.assertEqual(len(raced), 1)
        self.assertEqual(first["inserted_events"] + final["inserted_events"], 21)
        self.assertTrue(final["complete"])
        self.assertTrue(reconciliation["database_ready"], reconciliation)
        self.assertEqual(projection.event_count(self.install_id), 21)


class EventStoreKeySanitizationTests(EventStoreTestCase):
    def test_typed_key_identities_become_content_free_classes(self) -> None:
        item = {
            "summary": "keyboard event rejected",
            "reason": "KeyPress(KeyI)",
            "data": {
                "event_type": "KeyPress(Slash)",
                "release": "KeyRelease(Num4)",
                "key": "Some(Num4)",
                "target_key": "Some(ShiftLeft)",
                "hotkey_event": "KeyRelease(MetaRight)",
                "last_key": "SemiColon",
                "unknown_key": "Some(Unknown(57))",
                "event_name": 'Some("a")',
                "hold_state": "Some(Idle)",
                "error_code": "Some(Timeout)",
                "mode": "KeyA",
                "list": [{"key": "KeyB"}],
                "count": 3,
            },
        }

        sanitized, changed = store_module.sanitize_event(item)

        self.assertEqual(changed, 8)
        self.assertEqual(sanitized["reason"], "KeyPress(non_hotkey_key)")
        data = sanitized["data"]
        self.assertEqual(data["event_type"], "KeyPress(non_hotkey_key)")
        self.assertEqual(data["release"], "KeyRelease(non_hotkey_key)")
        self.assertEqual(data["key"], "Some(non_hotkey_key)")
        self.assertEqual(data["target_key"], "Some(ShiftLeft)")
        self.assertEqual(data["hotkey_event"], "KeyRelease(MetaRight)")
        self.assertEqual(data["last_key"], "non_hotkey_key")
        self.assertEqual(data["unknown_key"], "Some(non_hotkey_key)")
        self.assertEqual(data["event_name"], "Some(redacted_text)")
        self.assertEqual(data["hold_state"], "Some(Idle)")
        self.assertEqual(data["error_code"], "Some(Timeout)")
        self.assertEqual(data["mode"], "KeyA")
        self.assertEqual(data["list"], [{"key": "non_hotkey_key"}])
        self.assertEqual(item["data"]["key"], "Some(Num4)")
        for hotkey in ("Function", "F13", "ControlRight", "Alt", "AltGr"):
            self.assertEqual(
                store_module.sanitize_event({"data": {"key": "Some(%s)" % hotkey}})[1], 0
            )

    def test_embedded_forms_summaries_and_typed_characters_are_redacted(self) -> None:
        cases = [
            (
                {"summary": "rejected KeyPress(Num4) in listener"},
                {"summary": "rejected KeyPress(non_hotkey_key) in listener"},
            ),
            (
                {"summary": "saw Some(KeyQ) then KeyRelease(Unknown(57)), kept KeyPress(ShiftLeft)"},
                {
                    "summary": "saw Some(non_hotkey_key) then "
                    "KeyRelease(non_hotkey_key), kept KeyPress(ShiftLeft)"
                },
            ),
            ({"data": {"event_name": "a"}}, {"data": {"event_name": "redacted_text"}}),
            ({"data": {"event_name": "None"}}, {"data": {"event_name": "None"}}),
            (
                {"stream": "keyboard", "data": {"name": "b", "text": "hunter2", "char": "x"}},
                {
                    "stream": "keyboard",
                    "data": {
                        "name": "redacted_text",
                        "text": "redacted_text",
                        "char": "redacted_text",
                    },
                },
            ),
        ]
        for original, expected in cases:
            with self.subTest(original=original):
                self.assertEqual(store_module.sanitize_event(original)[0], expected)
        untouched = [
            {"summary": "/models/KeyA/ KeyA-7B MyKeyPress(KeyA) KeyPress(KeyAB) Some(Idle)"},
            {"stream": "system", "data": {"name": "parakeet", "text": "ok"}},
            {"data": {"target_key": "Some(ShiftLeft)", "hold_state": "Some(Idle)"}},
            {"data": {"note": "KeyRelease(MetaRight) Some(F13) Some(Function)"}},
        ]
        for item in untouched:
            with self.subTest(item=item):
                self.assertEqual(store_module.sanitize_event(item), (item, 0))
        once = store_module.sanitize_event(cases[0][0])[0]
        self.assertEqual(store_module.sanitize_event(once)[1], 0)

    def test_ingest_stores_only_sanitized_form_in_database_and_raw_archive(self) -> None:
        item = event("keyboard event rejected")
        item["data"] = {"event_type": "KeyPress(KeyI)", "key": "Some(KeyI)"}
        twin = event("keyboard event rejected")
        twin["data"] = {"event_type": "KeyPress(KeyJ)", "key": "Some(KeyJ)"}

        result = self.ingest([item, twin])

        self.assertEqual((result.inserted, result.duplicates), (1, 1))
        raw = self.archive.read_text()
        self.assertNotIn("KeyI", raw)
        self.assertNotIn("KeyJ", raw)
        self.assertIn("KeyPress(non_hotkey_key)", raw)
        page = self.store.query_events(store_module.EventQuery(install_id=self.install_id))
        self.assertEqual(page.events[0]["data"]["key"], "Some(non_hotkey_key)")


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
    def test_operator_cli_integrity_and_disable_dashboard(self) -> None:
        self.store.set_dashboard_ready(True)
        integrity_output = io.StringIO()
        with redirect_stdout(integrity_output):
            result = store_module.main(
                ["--root", str(self.root), "integrity"]
            )
        integrity = json.loads(integrity_output.getvalue())

        disable_output = io.StringIO()
        with redirect_stdout(disable_output):
            disabled_result = store_module.main(
                ["--root", str(self.root), "disable-dashboard"]
            )
        disabled = json.loads(disable_output.getvalue())

        self.assertEqual((result, disabled_result), (0, 0))
        self.assertEqual(integrity["schema"], "murmur-event-integrity/v1")
        self.assertEqual(integrity["result"], "ok")
        self.assertEqual(integrity["schema_version"], store_module.SCHEMA_VERSION)
        self.assertEqual(disabled, {"dashboard_reads_enabled": False})
        self.assertFalse(self.store.is_dashboard_ready())

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
                "app_version": "1.2.3",
            },
            received_at=1_786_000_000,
        )
        self.store.update_state(self.install_id, normalized)

        self.assertNotIn("app_version", normalized)

        install = self.store.get_install(self.install_id)
        assert install is not None
        self.assertEqual(install["state"]["input_device_count"], 2)
        self.assertNotIn("default_input", install["state"])

        with self.assertRaises(store_module.StoreError):
            self.store.update_state(self.install_id, {})
        with self.assertRaises(store_module.StoreError):
            self.store.update_state(
                self.install_id,
                {
                    **normalized,
                    "received_at": float("nan"),
                },
            )


if __name__ == "__main__":
    unittest.main()
