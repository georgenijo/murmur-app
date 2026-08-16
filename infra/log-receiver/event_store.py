#!/usr/bin/env python3
"""Bounded SQLite query store for the Murmur diagnostic-log receiver.

The raw per-install JSONL files remain the recovery and export source. This
module maintains an idempotent indexed projection, resumable backfill
checkpoints, and a persisted readiness gate for historical dashboard reads.
It intentionally uses only the Python standard library.
"""

from __future__ import annotations

import argparse
import base64
import hashlib
import hmac
import json
import math
import os
import re
import secrets
import shutil
import sqlite3
import sys
import tempfile
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Sequence
from zoneinfo import ZoneInfo


SCHEMA_VERSION = 1
DEFAULT_BUSY_TIMEOUT_MS = 2_000
DEFAULT_DATABASE_QUOTA_BYTES = 10 * 1024 * 1024 * 1024
MAX_EVENT_BYTES = 1024 * 1024
MAX_QUERY_LIMIT = 200
MAX_QUERY_RESULT_BYTES = 4 * 1024 * 1024
MAX_SEARCH_BYTES = 200
MAX_SEARCH_TERMS = 12
MAX_FILTER_BYTES = 80
MAX_CURSOR_BYTES = 1024
MAX_BACKFILL_LINES = 100_000
MAX_BACKFILL_BATCH = 500
MAX_BACKFILL_BATCH_BYTES = 4 * 1024 * 1024
INSTALL_ID_RE = re.compile(r"^[0-9a-fA-F-]{8,64}$")
APP_VERSION_RE = re.compile(r"^[0-9A-Za-z][0-9A-Za-z.+-]{0,39}$")
STREAM_RE = re.compile(r"^[0-9A-Za-z_.:-]{1,40}$")
LEVELS = frozenset(("trace", "debug", "info", "warn", "error"))
EASTERN = ZoneInfo("America/New_York")
MAX_TIME_US = 4_102_444_800_000_000  # 2100-01-01T00:00:00Z


class StoreError(RuntimeError):
    """Base class for fail-closed store errors."""


class StoreBusy(StoreError):
    pass


class StoreQuota(StoreError):
    pass


class StoreCorrupt(StoreError):
    pass


class StoreCommitError(StoreError):
    pass


class ArchiveError(StoreError):
    pass


class InvalidEvent(StoreError):
    pass


class InvalidQuery(StoreError):
    pass


@dataclass(frozen=True)
class PreparedEvent:
    event: dict
    canonical: bytes
    event_hash: str
    timestamp_text: str
    timestamp_us: int | None
    app_version: str
    stream: str
    level: str
    summary: str


@dataclass(frozen=True)
class IngestResult:
    received: int
    inserted: int
    duplicates: int


@dataclass(frozen=True)
class EventQuery:
    install_id: str | None = None
    start_us: int | None = None
    end_us: int | None = None
    app_version: str | None = None
    level: str | None = None
    stream: str | None = None
    search: str | None = None
    problems_only: bool = False
    limit: int = 100


@dataclass(frozen=True)
class EventPage:
    events: tuple[dict, ...]
    next_position: tuple[int, int] | None


def _raise_json_constant(value: str) -> None:
    raise ValueError("non-finite JSON value: %s" % value)


def parse_event_line(raw: bytes) -> dict:
    """Parse one bounded JSON object without accepting NaN or scalar values."""
    if not raw or len(raw) > MAX_EVENT_BYTES:
        raise InvalidEvent("event line is empty or too large")
    try:
        event = json.loads(raw, parse_constant=_raise_json_constant)
    except (UnicodeDecodeError, ValueError, RecursionError) as error:
        raise InvalidEvent("event line is not valid JSON") from error
    if not isinstance(event, dict):
        raise InvalidEvent("event line must be a JSON object")
    return event


def canonical_event(event: dict) -> bytes:
    if not isinstance(event, dict):
        raise InvalidEvent("event must be an object")
    try:
        encoded = json.dumps(
            event,
            ensure_ascii=False,
            allow_nan=False,
            sort_keys=True,
            separators=(",", ":"),
        ).encode("utf-8")
    except (TypeError, ValueError, RecursionError) as error:
        raise InvalidEvent("event is not bounded JSON") from error
    if len(encoded) > MAX_EVENT_BYTES:
        raise InvalidEvent("event is too large")
    return encoded


def event_hash(install_id: str, canonical: bytes) -> str:
    digest = hashlib.sha256()
    digest.update(install_id.lower().encode("ascii"))
    digest.update(b"\0")
    digest.update(canonical)
    return digest.hexdigest()


def timestamp_us(value: object) -> int | None:
    if not isinstance(value, str) or len(value) > 80:
        return None
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except (TypeError, ValueError):
        return None
    if parsed.tzinfo is None:
        return None
    try:
        value_us = int(parsed.astimezone(timezone.utc).timestamp() * 1_000_000)
        return value_us if 0 <= value_us <= MAX_TIME_US else None
    except (OverflowError, OSError, ValueError):
        return None


def _bounded_text(value: object, maximum: int) -> str:
    return value[:maximum] if isinstance(value, str) else ""


def prepare_event(install_id: str, event: dict) -> PreparedEvent:
    canonical = canonical_event(event)
    timestamp_text = _bounded_text(event.get("timestamp"), 80)
    return PreparedEvent(
        event=event,
        canonical=canonical,
        event_hash=event_hash(install_id, canonical),
        timestamp_text=timestamp_text,
        timestamp_us=timestamp_us(timestamp_text),
        app_version=_bounded_text(event.get("ingest_app_version"), 40) or "unknown",
        stream=_bounded_text(event.get("stream"), 40),
        level=_bounded_text(event.get("level"), 16),
        summary=_bounded_text(event.get("summary"), 2048),
    )


def normalize_install_id(value: str) -> str:
    if not isinstance(value, str) or not INSTALL_ID_RE.fullmatch(value):
        raise StoreError("invalid install id")
    return value.lower()


def normalize_state_snapshot(value: object, *, received_at: float | None = None) -> dict:
    """Accept only the current privacy-safe aggregate microphone contract."""
    if not isinstance(value, dict):
        raise StoreError("state must be an object")
    required = {
        "default_input_available",
        "input_device_count",
        "input_device_count_capped",
        "input_enumeration_ok",
    }
    keys = set(value)
    if not required.issubset(keys) or not keys.issubset(required | {"app_version"}):
        raise StoreError("state does not match aggregate contract")
    app_version = value.get("app_version")
    if app_version is not None and (
        not isinstance(app_version, str) or not APP_VERSION_RE.fullmatch(app_version)
    ):
        raise StoreError("state has invalid app version")
    count = value["input_device_count"]
    if (
        not isinstance(value["default_input_available"], bool)
        or not isinstance(count, int)
        or isinstance(count, bool)
        or not 0 <= count <= 256
        or not isinstance(value["input_device_count_capped"], bool)
        or not isinstance(value["input_enumeration_ok"], bool)
    ):
        raise StoreError("state has invalid aggregate values")
    observed_at = time.time() if received_at is None else received_at
    if (
        not isinstance(observed_at, (int, float))
        or isinstance(observed_at, bool)
        or not math.isfinite(observed_at)
        or not 0 < observed_at <= MAX_TIME_US / 1_000_000
    ):
        raise StoreError("state has invalid receive time")
    normalized = {key: value[key] for key in required}
    normalized["received_at"] = float(observed_at)
    return normalized


def _sqlite_error(error: BaseException) -> StoreError:
    text = str(error).lower()
    code = getattr(error, "sqlite_errorcode", None)
    if code in (sqlite3.SQLITE_BUSY, sqlite3.SQLITE_LOCKED) or "locked" in text or "busy" in text:
        return StoreBusy("database is busy")
    if code == sqlite3.SQLITE_FULL or "database or disk is full" in text or "disk full" in text:
        return StoreQuota("database or disk quota is full")
    if code in (sqlite3.SQLITE_CORRUPT, sqlite3.SQLITE_NOTADB) or "malformed" in text or "not a database" in text:
        return StoreCorrupt("database integrity failure")
    return StoreError("database operation failed")


def _fsync_directory(path: str) -> None:
    descriptor = os.open(path, os.O_RDONLY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


class EventStore:
    def __init__(
        self,
        path: str | os.PathLike[str],
        *,
        busy_timeout_ms: int = DEFAULT_BUSY_TIMEOUT_MS,
        quota_bytes: int = DEFAULT_DATABASE_QUOTA_BYTES,
    ) -> None:
        if not 1 <= busy_timeout_ms <= 30_000:
            raise ValueError("busy timeout is out of bounds")
        if quota_bytes < 1024 * 1024:
            raise ValueError("database quota must be at least 1 MiB")
        self.path = os.fspath(path)
        self.busy_timeout_ms = busy_timeout_ms
        self.quota_bytes = quota_bytes

    def _connect(self, *, write: bool = True) -> sqlite3.Connection:
        connection = None
        try:
            connection = sqlite3.connect(
                self.path,
                timeout=self.busy_timeout_ms / 1000,
                isolation_level=None,
            )
            connection.row_factory = sqlite3.Row
            connection.execute("PRAGMA foreign_keys = ON")
            connection.execute("PRAGMA busy_timeout = %d" % self.busy_timeout_ms)
            connection.execute("PRAGMA temp_store = FILE")
            connection.execute("PRAGMA cache_size = -2048")
            connection.execute("PRAGMA mmap_size = 0")
            if write:
                connection.execute("PRAGMA synchronous = FULL")
                connection.execute("PRAGMA journal_size_limit = 16777216")
                page_size = int(connection.execute("PRAGMA page_size").fetchone()[0])
                connection.execute(
                    "PRAGMA max_page_count = %d"
                    % max(1, self.quota_bytes // page_size)
                )
            else:
                connection.execute("PRAGMA query_only = ON")
            return connection
        except sqlite3.Error as error:
            if connection is not None:
                connection.close()
            raise _sqlite_error(error) from error

    def initialize(self, *, check_integrity: bool = True) -> None:
        deadline = time.monotonic() + self.busy_timeout_ms / 1000
        while True:
            try:
                self._initialize_once(check_integrity=check_integrity)
                return
            except StoreBusy:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.05)

    def _initialize_once(self, *, check_integrity: bool) -> None:
        os.makedirs(os.path.dirname(self.path) or ".", exist_ok=True)
        connection = self._connect()
        try:
            mode = connection.execute("PRAGMA journal_mode = WAL").fetchone()[0]
            if str(mode).lower() != "wal":
                raise StoreError("WAL mode is unavailable")
            connection.execute("PRAGMA synchronous = FULL")
            connection.execute("PRAGMA journal_size_limit = 16777216")
            page_size = int(connection.execute("PRAGMA page_size").fetchone()[0])
            connection.execute(
                "PRAGMA max_page_count = %d" % max(1, self.quota_bytes // page_size)
            )
            self._migrate(connection)
            if check_integrity:
                result = connection.execute("PRAGMA quick_check").fetchone()[0]
                if result != "ok":
                    raise StoreCorrupt("database quick_check failed")
        except sqlite3.Error as error:
            raise _sqlite_error(error) from error
        finally:
            connection.close()

    def _migrate(self, connection: sqlite3.Connection) -> None:
        try:
            connection.execute(
                """
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    version INTEGER PRIMARY KEY,
                    applied_at_us INTEGER NOT NULL
                )
                """
            )
            version_row = connection.execute(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations"
            ).fetchone()
            version = int(version_row[0])
            if version > SCHEMA_VERSION:
                raise StoreError("database schema is newer than this receiver")
            if version < 1:
                connection.executescript(
                    """
                    BEGIN IMMEDIATE;
                    CREATE TABLE IF NOT EXISTS installs (
                        install_id TEXT PRIMARY KEY,
                        device_name TEXT NOT NULL DEFAULT '',
                        os_version TEXT NOT NULL DEFAULT '',
                        hw_model TEXT NOT NULL DEFAULT '',
                        hw_specs TEXT NOT NULL DEFAULT '',
                        first_seen_us INTEGER NOT NULL,
                        last_seen_us INTEGER NOT NULL,
                        current_app_version TEXT NOT NULL DEFAULT '',
                        latest_state_json TEXT,
                        state_received_us INTEGER
                    ) WITHOUT ROWID;

                    CREATE TABLE IF NOT EXISTS events (
                        event_id INTEGER PRIMARY KEY AUTOINCREMENT,
                        install_id TEXT NOT NULL REFERENCES installs(install_id) ON DELETE RESTRICT,
                        event_hash TEXT NOT NULL,
                        event_timestamp TEXT NOT NULL DEFAULT '',
                        event_time_us INTEGER,
                        received_at_us INTEGER NOT NULL,
                        app_version TEXT NOT NULL DEFAULT '',
                        stream TEXT NOT NULL DEFAULT '',
                        level TEXT NOT NULL DEFAULT '',
                        summary TEXT NOT NULL DEFAULT '',
                        event_json TEXT NOT NULL,
                        UNIQUE (install_id, event_hash)
                    );

                    CREATE INDEX IF NOT EXISTS events_install_time
                        ON events(install_id, COALESCE(event_time_us, received_at_us) DESC, event_id DESC);
                    CREATE INDEX IF NOT EXISTS events_level_time
                        ON events(level, COALESCE(event_time_us, received_at_us) DESC, event_id DESC);
                    CREATE INDEX IF NOT EXISTS events_stream_time
                        ON events(stream, COALESCE(event_time_us, received_at_us) DESC, event_id DESC);
                    CREATE INDEX IF NOT EXISTS events_version_time
                        ON events(app_version, COALESCE(event_time_us, received_at_us) DESC, event_id DESC);

                    CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
                        summary,
                        content='events',
                        content_rowid='event_id',
                        tokenize='unicode61'
                    );
                    CREATE TRIGGER IF NOT EXISTS events_fts_insert AFTER INSERT ON events BEGIN
                        INSERT INTO events_fts(rowid, summary) VALUES (new.event_id, new.summary);
                    END;
                    CREATE TRIGGER IF NOT EXISTS events_fts_delete AFTER DELETE ON events BEGIN
                        INSERT INTO events_fts(events_fts, rowid, summary)
                        VALUES ('delete', old.event_id, old.summary);
                    END;
                    CREATE TRIGGER IF NOT EXISTS events_fts_update AFTER UPDATE OF summary ON events BEGIN
                        INSERT INTO events_fts(events_fts, rowid, summary)
                        VALUES ('delete', old.event_id, old.summary);
                        INSERT INTO events_fts(rowid, summary) VALUES (new.event_id, new.summary);
                    END;

                    CREATE TABLE IF NOT EXISTS store_metadata (
                        key TEXT PRIMARY KEY,
                        value TEXT NOT NULL
                    ) WITHOUT ROWID;
                    INSERT OR IGNORE INTO store_metadata(key, value)
                    VALUES ('dashboard_ready', '0');

                    CREATE TABLE IF NOT EXISTS backfill_checkpoints (
                        source_path TEXT PRIMARY KEY,
                        install_id TEXT NOT NULL REFERENCES installs(install_id) ON DELETE RESTRICT,
                        byte_offset INTEGER NOT NULL DEFAULT 0,
                        raw_lines INTEGER NOT NULL DEFAULT 0,
                        valid_objects INTEGER NOT NULL DEFAULT 0,
                        malformed_lines INTEGER NOT NULL DEFAULT 0,
                        duplicate_events INTEGER NOT NULL DEFAULT 0,
                        inserted_events INTEGER NOT NULL DEFAULT 0,
                        earliest_time_us INTEGER,
                        latest_time_us INTEGER,
                        source_size INTEGER NOT NULL DEFAULT 0,
                        source_mtime_ns INTEGER NOT NULL DEFAULT 0,
                        complete INTEGER NOT NULL DEFAULT 0 CHECK (complete IN (0, 1)),
                        updated_at_us INTEGER NOT NULL
                    ) WITHOUT ROWID;
                    """
                )
                connection.execute(
                    "INSERT OR IGNORE INTO store_metadata(key, value) VALUES ('cursor_secret', ?)",
                    (secrets.token_hex(32),),
                )
                connection.execute(
                    "INSERT OR IGNORE INTO schema_migrations(version, applied_at_us) VALUES (?, ?)",
                    (1, int(time.time() * 1_000_000)),
                )
                connection.execute("COMMIT")
            else:
                connection.execute(
                    "INSERT OR IGNORE INTO store_metadata(key, value) VALUES ('cursor_secret', ?)",
                    (secrets.token_hex(32),),
                )
        except BaseException:
            try:
                connection.execute("ROLLBACK")
            except sqlite3.Error:
                pass
            raise

    def schema_version(self) -> int:
        connection = self._connect(write=False)
        try:
            row = connection.execute(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations"
            ).fetchone()
            return int(row[0])
        except sqlite3.Error as error:
            raise _sqlite_error(error) from error
        finally:
            connection.close()

    def integrity_check(self) -> str:
        connection = self._connect(write=False)
        try:
            rows = connection.execute("PRAGMA integrity_check").fetchall()
            return "ok" if len(rows) == 1 and rows[0][0] == "ok" else "failed"
        except sqlite3.Error as error:
            raise _sqlite_error(error) from error
        finally:
            connection.close()

    def is_dashboard_ready(self) -> bool:
        connection = self._connect(write=False)
        try:
            row = connection.execute(
                "SELECT value FROM store_metadata WHERE key = 'dashboard_ready'"
            ).fetchone()
            return bool(row and row[0] == "1")
        except sqlite3.Error as error:
            raise _sqlite_error(error) from error
        finally:
            connection.close()

    def cursor_secret(self) -> str:
        connection = self._connect(write=False)
        try:
            row = connection.execute(
                "SELECT value FROM store_metadata WHERE key = 'cursor_secret'"
            ).fetchone()
            if not row or not isinstance(row[0], str) or len(row[0]) != 64:
                raise StoreCorrupt("cursor secret is missing")
            return row[0]
        except sqlite3.Error as error:
            raise _sqlite_error(error) from error
        finally:
            connection.close()

    def set_dashboard_ready(self, ready: bool) -> None:
        connection = self._connect()
        try:
            connection.execute("BEGIN IMMEDIATE")
            connection.execute(
                "INSERT INTO store_metadata(key, value) VALUES ('dashboard_ready', ?) "
                "ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                ("1" if ready else "0",),
            )
            self._commit(connection)
        except sqlite3.Error as error:
            try:
                connection.execute("ROLLBACK")
            except sqlite3.Error:
                pass
            raise _sqlite_error(error) from error
        finally:
            connection.close()

    def _commit(self, connection: sqlite3.Connection) -> None:
        try:
            connection.execute("COMMIT")
        except sqlite3.Error as error:
            raise StoreCommitError("database commit failed") from error

    @staticmethod
    def _metadata(metadata: dict | None) -> dict:
        metadata = metadata if isinstance(metadata, dict) else {}
        return {
            "device_name": _bounded_text(metadata.get("device_name"), 120),
            "os_version": _bounded_text(metadata.get("os"), 120),
            "hw_model": _bounded_text(metadata.get("hw"), 120),
            "hw_specs": _bounded_text(metadata.get("specs"), 120),
            "current_app_version": _bounded_text(metadata.get("last_version"), 120),
        }

    def _upsert_install(
        self,
        connection: sqlite3.Connection,
        install_id: str,
        metadata: dict | None,
        seen_us: int,
    ) -> None:
        values = self._metadata(metadata)
        connection.execute(
            """
            INSERT INTO installs(
                install_id, device_name, os_version, hw_model, hw_specs,
                first_seen_us, last_seen_us, current_app_version
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            ON CONFLICT(install_id) DO UPDATE SET
                device_name = CASE WHEN excluded.device_name != '' THEN excluded.device_name ELSE installs.device_name END,
                os_version = CASE WHEN excluded.os_version != '' THEN excluded.os_version ELSE installs.os_version END,
                hw_model = CASE WHEN excluded.hw_model != '' THEN excluded.hw_model ELSE installs.hw_model END,
                hw_specs = CASE WHEN excluded.hw_specs != '' THEN excluded.hw_specs ELSE installs.hw_specs END,
                last_seen_us = MAX(installs.last_seen_us, excluded.last_seen_us),
                current_app_version = CASE WHEN excluded.current_app_version != '' THEN excluded.current_app_version ELSE installs.current_app_version END
            """,
            (
                install_id,
                values["device_name"],
                values["os_version"],
                values["hw_model"],
                values["hw_specs"],
                seen_us,
                seen_us,
                values["current_app_version"],
            ),
        )

    @staticmethod
    def _insert_prepared(
        connection: sqlite3.Connection,
        install_id: str,
        prepared: PreparedEvent,
        received_at_us: int,
    ) -> bool:
        cursor = connection.execute(
            """
            INSERT OR IGNORE INTO events(
                install_id, event_hash, event_timestamp, event_time_us,
                received_at_us, app_version, stream, level, summary, event_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                install_id,
                prepared.event_hash,
                prepared.timestamp_text,
                prepared.timestamp_us,
                received_at_us,
                prepared.app_version,
                prepared.stream,
                prepared.level,
                prepared.summary,
                prepared.canonical.decode("utf-8"),
            ),
        )
        return cursor.rowcount == 1

    def _logical_database_bytes(self, connection: sqlite3.Connection) -> int:
        page_count = int(connection.execute("PRAGMA page_count").fetchone()[0])
        page_size = int(connection.execute("PRAGMA page_size").fetchone()[0])
        return page_count * page_size

    @staticmethod
    def _append_archive(
        path: str,
        lines: Sequence[bytes],
        max_bytes: int | None = None,
    ) -> tuple[int, bool, bool] | None:
        if not lines:
            return None
        parent = os.path.dirname(path)
        parent_existed = os.path.isdir(parent)
        existed = os.path.exists(path)
        original_size = os.path.getsize(path) if existed else 0
        append_bytes = sum(len(line) + 1 for line in lines)
        if max_bytes is not None and original_size + append_bytes > max_bytes:
            raise StoreQuota("raw archive quota exceeded")
        os.makedirs(parent, exist_ok=True)
        try:
            with open(path, "ab") as handle:
                handle.write(b"\n".join(lines) + b"\n")
                handle.flush()
                os.fsync(handle.fileno())
            _fsync_directory(parent)
            return original_size, existed, parent_existed
        except OSError as error:
            checkpoint = (original_size, existed, parent_existed)
            try:
                EventStore._restore_archive(path, checkpoint)
            except ArchiveError as rollback_error:
                raise rollback_error from error
            raise ArchiveError("raw archive append failed") from error

    @staticmethod
    def _restore_archive(
        path: str, checkpoint: tuple[int, bool, bool] | None
    ) -> None:
        if checkpoint is None:
            return
        original_size, existed, parent_existed = checkpoint
        parent = os.path.dirname(path)
        try:
            if not existed and original_size == 0:
                if os.path.exists(path):
                    os.unlink(path)
            else:
                with open(path, "r+b") as handle:
                    handle.truncate(original_size)
                    handle.flush()
                    os.fsync(handle.fileno())
            _fsync_directory(parent)
            if not parent_existed:
                try:
                    os.rmdir(parent)
                except OSError:
                    pass
        except OSError as error:
            raise ArchiveError("raw archive rollback failed") from error

    def ingest_batch(
        self,
        install_id: str,
        events: Sequence[dict],
        *,
        metadata: dict | None,
        archive_path: str,
        archive_quota_bytes: int | None = None,
        received_at: float | None = None,
    ) -> IngestResult:
        install_id = normalize_install_id(install_id)
        if not events:
            raise InvalidEvent("empty batch")
        prepared = [prepare_event(install_id, event) for event in events]
        received_at_us = int(
            (time.time() if received_at is None else received_at) * 1_000_000
        )
        connection = self._connect()
        archive_checkpoint = None
        inserted_lines: list[bytes] = []
        try:
            connection.execute("BEGIN IMMEDIATE")
            self._upsert_install(connection, install_id, metadata, received_at_us)
            for item in prepared:
                if self._insert_prepared(connection, install_id, item, received_at_us):
                    inserted_lines.append(item.canonical)
            if self._logical_database_bytes(connection) > self.quota_bytes:
                raise StoreQuota("database quota exceeded")
            archive_checkpoint = self._append_archive(
                archive_path, inserted_lines, archive_quota_bytes
            )
            self._commit(connection)
        except BaseException as error:
            try:
                connection.execute("ROLLBACK")
            except sqlite3.Error:
                pass
            try:
                self._restore_archive(archive_path, archive_checkpoint)
            except ArchiveError as rollback_error:
                raise rollback_error from error
            if isinstance(error, StoreError):
                raise
            if isinstance(error, sqlite3.Error):
                raise _sqlite_error(error) from error
            raise
        finally:
            connection.close()
        inserted = len(inserted_lines)
        return IngestResult(len(events), inserted, len(events) - inserted)

    def update_state(
        self,
        install_id: str,
        state: dict,
        *,
        metadata: dict | None = None,
    ) -> None:
        install_id = normalize_install_id(install_id)
        if not isinstance(state, dict) or "received_at" not in state:
            raise StoreError("state does not match aggregate contract")
        received_at = state.get("received_at")
        state = normalize_state_snapshot(
            {key: value for key, value in state.items() if key != "received_at"},
            received_at=received_at,
        )
        seen_us = int(state["received_at"] * 1_000_000)
        state_json = json.dumps(
            state, ensure_ascii=False, sort_keys=True, separators=(",", ":")
        )
        connection = self._connect()
        try:
            connection.execute("BEGIN IMMEDIATE")
            self._upsert_install(connection, install_id, metadata, seen_us)
            connection.execute(
                "UPDATE installs SET latest_state_json = ?, state_received_us = ? WHERE install_id = ?",
                (state_json, seen_us, install_id),
            )
            self._commit(connection)
        except BaseException as error:
            try:
                connection.execute("ROLLBACK")
            except sqlite3.Error:
                pass
            if isinstance(error, StoreError):
                raise
            if isinstance(error, sqlite3.Error):
                raise _sqlite_error(error) from error
            raise
        finally:
            connection.close()

    def list_installs(self) -> list[dict]:
        connection = self._connect(write=False)
        try:
            rows = connection.execute(
                """
                SELECT i.*,
                       COUNT(e.event_id) AS event_count,
                       MAX(e.event_time_us) AS latest_event_us,
                       MAX(e.received_at_us) AS latest_received_us
                FROM installs i
                LEFT JOIN events e ON e.install_id = i.install_id
                GROUP BY i.install_id
                HAVING COUNT(e.event_id) > 0
                ORDER BY COALESCE(MAX(e.received_at_us), i.last_seen_us) DESC, i.install_id
                LIMIT 150
                """
            ).fetchall()
            return [self._install_row(row) for row in rows]
        except sqlite3.Error as error:
            raise _sqlite_error(error) from error
        finally:
            connection.close()

    def get_install(self, install_id: str) -> dict | None:
        install_id = normalize_install_id(install_id)
        connection = self._connect(write=False)
        try:
            row = connection.execute(
                """
                SELECT i.*,
                       COUNT(e.event_id) AS event_count,
                       MAX(e.event_time_us) AS latest_event_us,
                       MAX(e.received_at_us) AS latest_received_us
                FROM installs i
                LEFT JOIN events e ON e.install_id = i.install_id
                WHERE i.install_id = ?
                GROUP BY i.install_id
                """,
                (install_id,),
            ).fetchone()
            return self._install_row(row) if row else None
        except sqlite3.Error as error:
            raise _sqlite_error(error) from error
        finally:
            connection.close()

    @staticmethod
    def _install_row(row: sqlite3.Row) -> dict:
        state = {}
        if row["latest_state_json"]:
            try:
                state = json.loads(row["latest_state_json"])
            except ValueError:
                state = {}
        return {
            "id": row["install_id"],
            "device": row["device_name"],
            "os": row["os_version"],
            "hw": row["hw_model"],
            "specs": row["hw_specs"],
            "version": row["current_app_version"] or "?",
            "events": int(row["event_count"]),
            "first_seen": row["first_seen_us"] / 1_000_000,
            "mtime": (row["latest_received_us"] or row["last_seen_us"]) / 1_000_000,
            "last_event_us": row["latest_event_us"],
            "state": state,
        }

    @staticmethod
    def _fts_query(search: str) -> str:
        if not isinstance(search, str) or not search.strip():
            raise InvalidQuery("search is empty")
        if len(search.encode("utf-8")) > MAX_SEARCH_BYTES:
            raise InvalidQuery("search is too long")
        terms = re.findall(r"[\w.-]+", search, flags=re.UNICODE)
        if not terms or len(terms) > MAX_SEARCH_TERMS:
            raise InvalidQuery("search must contain 1-12 terms")
        return " AND ".join('"%s"' % term.replace('"', '""') for term in terms)

    @staticmethod
    def validate_query(query: EventQuery) -> EventQuery:
        install_id = normalize_install_id(query.install_id) if query.install_id else None
        if query.limit < 1 or query.limit > MAX_QUERY_LIMIT:
            raise InvalidQuery("query limit is out of bounds")
        if query.start_us is not None and query.end_us is not None and query.start_us > query.end_us:
            raise InvalidQuery("date range is reversed")
        if query.app_version and not APP_VERSION_RE.fullmatch(query.app_version):
            raise InvalidQuery("invalid app version")
        if query.level and query.level not in LEVELS:
            raise InvalidQuery("invalid level")
        if query.stream and not STREAM_RE.fullmatch(query.stream):
            raise InvalidQuery("invalid stream")
        if query.search:
            EventStore._fts_query(query.search)
        return EventQuery(
            install_id=install_id,
            start_us=query.start_us,
            end_us=query.end_us,
            app_version=query.app_version,
            level=query.level,
            stream=query.stream,
            search=query.search,
            problems_only=query.problems_only,
            limit=query.limit,
        )

    def query_events(
        self,
        query: EventQuery,
        *,
        before: tuple[int, int] | None = None,
    ) -> EventPage:
        query = self.validate_query(query)
        sort_time = "COALESCE(e.event_time_us, e.received_at_us)"
        clauses = []
        values: list[object] = []
        join = ""
        if query.install_id:
            clauses.append("e.install_id = ?")
            values.append(query.install_id)
        if query.start_us is not None:
            clauses.append("%s >= ?" % sort_time)
            values.append(query.start_us)
        if query.end_us is not None:
            clauses.append("%s <= ?" % sort_time)
            values.append(query.end_us)
        if query.app_version:
            clauses.append("e.app_version = ?")
            values.append(query.app_version)
        if query.level:
            clauses.append("e.level = ?")
            values.append(query.level)
        if query.stream:
            clauses.append("e.stream = ?")
            values.append(query.stream)
        if query.problems_only:
            clauses.append("e.level IN ('warn', 'error')")
        if query.search:
            join = " JOIN events_fts f ON f.rowid = e.event_id"
            clauses.append("f.summary MATCH ?")
            values.append(self._fts_query(query.search))
        if before is not None:
            if (
                not isinstance(before, tuple)
                or len(before) != 2
                or not all(isinstance(item, int) and item >= 0 for item in before)
            ):
                raise InvalidQuery("invalid cursor position")
            clauses.append(
                "(%s < ? OR (%s = ? AND e.event_id < ?))"
                % (sort_time, sort_time)
            )
            values.extend((before[0], before[0], before[1]))
        values.append(query.limit + 1)
        sql = (
            "SELECT e.event_id, e.install_id, %s AS sort_time_us, e.event_json "
            "FROM events e%s WHERE %s "
            "ORDER BY %s DESC, e.event_id DESC LIMIT ?"
            % (sort_time, join, " AND ".join(clauses) or "1", sort_time)
        )
        connection = self._connect(write=False)
        rows = []
        result_bytes = 0
        budget_exhausted = False
        try:
            cursor = connection.execute(sql, values)
            for row in cursor:
                row_bytes = len(row["event_json"].encode("utf-8"))
                if rows and result_bytes + row_bytes > MAX_QUERY_RESULT_BYTES:
                    budget_exhausted = True
                    break
                rows.append(row)
                result_bytes += row_bytes
        except sqlite3.Error as error:
            raise _sqlite_error(error) from error
        finally:
            connection.close()
        has_more = budget_exhausted or len(rows) > query.limit
        rows = rows[: query.limit]
        events = []
        for row in rows:
            try:
                event = json.loads(row["event_json"])
            except ValueError as error:
                raise StoreCorrupt("stored event JSON is invalid") from error
            event["_store_install_id"] = row["install_id"]
            event["_store_event_id"] = row["event_id"]
            events.append(event)
        next_position = None
        if has_more and rows:
            last = rows[-1]
            next_position = (int(last["sort_time_us"]), int(last["event_id"]))
        return EventPage(tuple(events), next_position)

    def event_count(self, install_id: str) -> int:
        install_id = normalize_install_id(install_id)
        connection = self._connect(write=False)
        try:
            return int(
                connection.execute(
                    "SELECT COUNT(*) FROM events WHERE install_id = ?", (install_id,)
                ).fetchone()[0]
            )
        except sqlite3.Error as error:
            raise _sqlite_error(error) from error
        finally:
            connection.close()

    def _checkpoint(
        self, connection: sqlite3.Connection, source_path: str
    ) -> sqlite3.Row | None:
        return connection.execute(
            "SELECT * FROM backfill_checkpoints WHERE source_path = ?", (source_path,)
        ).fetchone()

    def import_backfill_chunk(
        self,
        install_id: str,
        source_path: str,
        *,
        events: Sequence[dict],
        raw_lines: int,
        malformed_lines: int,
        start_offset: int,
        end_offset: int,
        source_size: int,
        source_mtime_ns: int,
        complete: bool,
        metadata: dict | None = None,
    ) -> IngestResult:
        install_id = normalize_install_id(install_id)
        prepared = [prepare_event(install_id, event) for event in events]
        seen_us = max(1, source_mtime_ns // 1_000)
        updated_at_us = int(time.time() * 1_000_000)
        connection = self._connect()
        inserted = 0
        try:
            connection.execute("BEGIN IMMEDIATE")
            self._upsert_install(connection, install_id, metadata, seen_us)
            previous = self._checkpoint(connection, source_path)
            expected_offset = int(previous["byte_offset"]) if previous else 0
            if (
                start_offset != expected_offset
                or raw_lines < 1
                or malformed_lines < 0
                or raw_lines != len(prepared) + malformed_lines
                or not 0 <= start_offset < end_offset <= source_size
                or complete != (end_offset == source_size)
            ):
                raise StoreError("backfill chunk is not contiguous")
            for item in prepared:
                if self._insert_prepared(connection, install_id, item, seen_us):
                    inserted += 1
            earliest = min(
                (item.timestamp_us for item in prepared if item.timestamp_us is not None),
                default=None,
            )
            latest = max(
                (item.timestamp_us for item in prepared if item.timestamp_us is not None),
                default=None,
            )
            connection.execute(
                """
                INSERT INTO backfill_checkpoints(
                    source_path, install_id, byte_offset, raw_lines, valid_objects,
                    malformed_lines, duplicate_events, inserted_events,
                    earliest_time_us, latest_time_us, source_size, source_mtime_ns,
                    complete, updated_at_us
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                ON CONFLICT(source_path) DO UPDATE SET
                    byte_offset = excluded.byte_offset,
                    raw_lines = backfill_checkpoints.raw_lines + excluded.raw_lines,
                    valid_objects = backfill_checkpoints.valid_objects + excluded.valid_objects,
                    malformed_lines = backfill_checkpoints.malformed_lines + excluded.malformed_lines,
                    duplicate_events = backfill_checkpoints.duplicate_events + excluded.duplicate_events,
                    inserted_events = backfill_checkpoints.inserted_events + excluded.inserted_events,
                    earliest_time_us = CASE
                        WHEN backfill_checkpoints.earliest_time_us IS NULL THEN excluded.earliest_time_us
                        WHEN excluded.earliest_time_us IS NULL THEN backfill_checkpoints.earliest_time_us
                        ELSE MIN(backfill_checkpoints.earliest_time_us, excluded.earliest_time_us) END,
                    latest_time_us = CASE
                        WHEN backfill_checkpoints.latest_time_us IS NULL THEN excluded.latest_time_us
                        WHEN excluded.latest_time_us IS NULL THEN backfill_checkpoints.latest_time_us
                        ELSE MAX(backfill_checkpoints.latest_time_us, excluded.latest_time_us) END,
                    source_size = excluded.source_size,
                    source_mtime_ns = excluded.source_mtime_ns,
                    complete = excluded.complete,
                    updated_at_us = excluded.updated_at_us
                """,
                (
                    source_path,
                    install_id,
                    end_offset,
                    raw_lines,
                    len(prepared),
                    malformed_lines,
                    len(prepared) - inserted,
                    inserted,
                    earliest,
                    latest,
                    source_size,
                    source_mtime_ns,
                    1 if complete else 0,
                    updated_at_us,
                ),
            )
            if self._logical_database_bytes(connection) > self.quota_bytes:
                raise StoreQuota("database quota exceeded")
            self._commit(connection)
        except BaseException as error:
            try:
                connection.execute("ROLLBACK")
            except sqlite3.Error:
                pass
            if isinstance(error, StoreError):
                raise
            if isinstance(error, sqlite3.Error):
                raise _sqlite_error(error) from error
            raise
        finally:
            connection.close()
        return IngestResult(len(prepared), inserted, len(prepared) - inserted)

    def checkpoint_offset(self, source_path: str) -> int:
        connection = self._connect(write=False)
        try:
            row = self._checkpoint(connection, source_path)
            return int(row["byte_offset"]) if row else 0
        except sqlite3.Error as error:
            raise _sqlite_error(error) from error
        finally:
            connection.close()

    def checkpoint_reports(self) -> list[dict]:
        connection = self._connect(write=False)
        try:
            rows = connection.execute(
                "SELECT * FROM backfill_checkpoints ORDER BY install_id, source_path"
            ).fetchall()
            return [dict(row) for row in rows]
        except sqlite3.Error as error:
            raise _sqlite_error(error) from error
        finally:
            connection.close()

    def backup(self, destination: str) -> None:
        destination = os.path.abspath(destination)
        if destination == os.path.abspath(self.path):
            raise StoreError("backup destination must differ from database")
        os.makedirs(os.path.dirname(destination) or ".", exist_ok=True)
        temporary = destination + ".tmp-%d" % os.getpid()
        source = self._connect(write=False)
        target = sqlite3.connect(temporary)
        try:
            source.backup(target, pages=256, sleep=0.05)
            result = target.execute("PRAGMA integrity_check").fetchone()[0]
            if result != "ok":
                raise StoreCorrupt("backup integrity check failed")
            target.close()
            source.close()
            with open(temporary, "r+b") as handle:
                os.fsync(handle.fileno())
            os.replace(temporary, destination)
            _fsync_directory(os.path.dirname(destination) or ".")
        except BaseException:
            target.close()
            source.close()
            try:
                os.unlink(temporary)
            except OSError:
                pass
            raise


def encode_cursor(secret: str, query: EventQuery, position: tuple[int, int]) -> str:
    payload = {
        "v": 1,
        "q": query_fingerprint(query),
        "t": position[0],
        "i": position[1],
    }
    raw = json.dumps(payload, sort_keys=True, separators=(",", ":")).encode("utf-8")
    signature = hmac.new(secret.encode("utf-8"), raw, hashlib.sha256).digest()
    return base64.urlsafe_b64encode(raw + signature).decode("ascii").rstrip("=")


def decode_cursor(secret: str, query: EventQuery, cursor: str) -> tuple[int, int]:
    if not isinstance(cursor, str) or not cursor or len(cursor) > MAX_CURSOR_BYTES:
        raise InvalidQuery("invalid cursor")
    try:
        padding = "=" * (-len(cursor) % 4)
        blob = base64.b64decode(cursor + padding, altchars=b"-_", validate=True)
        raw, supplied = blob[:-32], blob[-32:]
        expected = hmac.new(secret.encode("utf-8"), raw, hashlib.sha256).digest()
        if len(supplied) != 32 or not hmac.compare_digest(supplied, expected):
            raise InvalidQuery("invalid cursor")
        payload = json.loads(raw)
    except (ValueError, TypeError, UnicodeDecodeError, json.JSONDecodeError) as error:
        raise InvalidQuery("invalid cursor") from error
    if (
        not isinstance(payload, dict)
        or set(payload) != {"v", "q", "t", "i"}
        or payload["v"] != 1
        or payload["q"] != query_fingerprint(query)
        or not isinstance(payload["t"], int)
        or not isinstance(payload["i"], int)
        or payload["t"] < 0
        or payload["i"] < 0
    ):
        raise InvalidQuery("invalid cursor")
    return payload["t"], payload["i"]


def query_fingerprint(query: EventQuery) -> str:
    raw = json.dumps(
        {
            "install": query.install_id,
            "start": query.start_us,
            "end": query.end_us,
            "version": query.app_version,
            "level": query.level,
            "stream": query.stream,
            "search": query.search,
            "problems": query.problems_only,
            "limit": query.limit,
        },
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(raw).hexdigest()[:24]


def parse_local_datetime(value: str, zone: str, *, end: bool = False) -> int | None:
    if value == "":
        return None
    if not isinstance(value, str) or len(value) > 32 or zone not in ("utc", "eastern"):
        raise InvalidQuery("invalid date filter")
    try:
        parsed = datetime.strptime(value, "%Y-%m-%dT%H:%M")
    except ValueError as error:
        raise InvalidQuery("date must use YYYY-MM-DDTHH:MM") from error
    parsed = parsed.replace(tzinfo=timezone.utc if zone == "utc" else EASTERN)
    if end:
        parsed = parsed.replace(second=59, microsecond=999999)
    value_us = int(parsed.astimezone(timezone.utc).timestamp() * 1_000_000)
    if not 0 <= value_us <= MAX_TIME_US:
        raise InvalidQuery("date is outside the supported range")
    return value_us


def _bounded_line(handle, remaining: int) -> tuple[bytes, int, bool]:
    """Read one line without retaining more than MAX_EVENT_BYTES."""
    if remaining <= 0:
        return b"", 0, False
    chunk = handle.readline(min(MAX_EVENT_BYTES + 2, remaining))
    consumed = len(chunk)
    if chunk.endswith(b"\n") or consumed == remaining:
        return chunk.rstrip(b"\r\n"), consumed, len(chunk) > MAX_EVENT_BYTES + 1
    oversized = True
    while consumed < remaining:
        piece = handle.readline(min(64 * 1024, remaining - consumed))
        if not piece:
            break
        consumed += len(piece)
        if piece.endswith(b"\n"):
            break
    return b"", consumed, oversized


def _load_metadata(root: Path, install_id: str) -> dict:
    path = root / install_id / "meta.json"
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}
    return value if isinstance(value, dict) else {}


def _load_state(root: Path, install_id: str) -> dict | None:
    path = root / install_id / "state.json"
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return None
    if not isinstance(value, dict):
        return None
    received_at = value.pop("received_at", None)
    if (
        not isinstance(received_at, (int, float))
        or isinstance(received_at, bool)
        or received_at <= 0
    ):
        return None
    try:
        return normalize_state_snapshot(value, received_at=float(received_at))
    except StoreError:
        return None


def discover_sources(root: Path) -> list[tuple[str, Path, str]]:
    sources = []
    if not root.is_dir():
        return sources
    for directory in sorted(root.iterdir()):
        if not directory.is_dir() or not INSTALL_ID_RE.fullmatch(directory.name):
            continue
        path = directory / "events.jsonl"
        if path.is_file():
            sources.append((directory.name.lower(), path, str(path.relative_to(root))))
    return sources


def backfill(
    store: EventStore,
    root: Path,
    *,
    max_lines: int,
    batch_size: int = MAX_BACKFILL_BATCH,
) -> dict:
    if not 1 <= max_lines <= MAX_BACKFILL_LINES:
        raise StoreError("max-lines is out of bounds")
    if not 1 <= batch_size <= MAX_BACKFILL_BATCH:
        raise StoreError("batch-size is out of bounds")
    processed = inserted = duplicates = malformed = 0
    sources = discover_sources(root)
    for install_id, path, relative in sources:
        if processed >= max_lines:
            break
        stat = path.stat()
        snapshot_size = stat.st_size
        offset = store.checkpoint_offset(relative)
        if offset > snapshot_size:
            raise StoreError("backfill source shrank; operator reset is required")
        metadata = _load_metadata(root, install_id)
        state = _load_state(root, install_id)
        if state is not None:
            store.update_state(install_id, state, metadata=metadata)
        with path.open("rb") as handle:
            handle.seek(offset)
            while offset < snapshot_size and processed < max_lines:
                batch_start = offset
                events: list[dict] = []
                raw_count = malformed_count = 0
                retained_bytes = 0
                batch_end = offset
                while (
                    batch_end < snapshot_size
                    and raw_count < batch_size
                    and processed + raw_count < max_lines
                ):
                    raw, consumed, oversized = _bounded_line(
                        handle, snapshot_size - batch_end
                    )
                    if consumed <= 0:
                        break
                    batch_end += consumed
                    raw_count += 1
                    if (
                        not oversized
                        and events
                        and retained_bytes + len(raw) > MAX_BACKFILL_BATCH_BYTES
                    ):
                        handle.seek(-consumed, os.SEEK_CUR)
                        batch_end -= consumed
                        raw_count -= 1
                        break
                    if oversized:
                        malformed_count += 1
                        continue
                    try:
                        events.append(parse_event_line(raw.strip()))
                        retained_bytes += len(raw)
                    except InvalidEvent:
                        malformed_count += 1
                if raw_count == 0:
                    break
                result = store.import_backfill_chunk(
                    install_id,
                    relative,
                    events=events,
                    raw_lines=raw_count,
                    malformed_lines=malformed_count,
                    start_offset=batch_start,
                    end_offset=batch_end,
                    source_size=snapshot_size,
                    source_mtime_ns=stat.st_mtime_ns,
                    complete=batch_end >= snapshot_size,
                    metadata=metadata,
                )
                offset = batch_end
                processed += raw_count
                inserted += result.inserted
                duplicates += result.duplicates
                malformed += malformed_count
    reports = store.checkpoint_reports()
    finished = {
        report["source_path"]
        for report in reports
        if report["complete"] == 1
        and report["byte_offset"] == report["source_size"]
    }
    complete = bool(sources) and all(
        relative in finished for _, _, relative in sources
    )
    return {
        "schema": "murmur-event-backfill/v1",
        "processed_lines": processed,
        "inserted_events": inserted,
        "duplicate_events": duplicates,
        "malformed_lines": malformed,
        "complete": complete,
    }


def _scan_reconciliation_source(
    install_id: str, path: Path, temporary_directory: str
) -> dict:
    raw_lines = valid_objects = malformed_lines = duplicates = untimed_events = 0
    earliest = latest = None
    dedupe_path = os.path.join(temporary_directory, "hashes.sqlite3")
    dedupe = sqlite3.connect(dedupe_path)
    dedupe.execute("PRAGMA journal_mode = OFF")
    dedupe.execute("PRAGMA synchronous = OFF")
    dedupe.execute("CREATE TABLE hashes(value TEXT PRIMARY KEY) WITHOUT ROWID")
    snapshot_size = path.stat().st_size
    offset = 0
    with path.open("rb") as handle:
        while offset < snapshot_size:
            raw, consumed, oversized = _bounded_line(handle, snapshot_size - offset)
            if consumed <= 0:
                break
            offset += consumed
            raw_lines += 1
            if oversized:
                malformed_lines += 1
                continue
            try:
                event = parse_event_line(raw.strip())
                prepared = prepare_event(install_id, event)
            except InvalidEvent:
                malformed_lines += 1
                continue
            valid_objects += 1
            if prepared.timestamp_us is None:
                untimed_events += 1
            cursor = dedupe.execute(
                "INSERT OR IGNORE INTO hashes(value) VALUES (?)", (prepared.event_hash,)
            )
            if cursor.rowcount != 1:
                duplicates += 1
            if prepared.timestamp_us is not None:
                earliest = (
                    prepared.timestamp_us if earliest is None else min(earliest, prepared.timestamp_us)
                )
                latest = (
                    prepared.timestamp_us if latest is None else max(latest, prepared.timestamp_us)
                )
    digest = hashlib.sha256()
    for row in dedupe.execute("SELECT value FROM hashes ORDER BY value"):
        digest.update(row[0].encode("ascii"))
        digest.update(b"\n")
    dedupe.close()
    return {
        "raw_lines": raw_lines,
        "valid_objects": valid_objects,
        "malformed_lines": malformed_lines,
        "duplicates": duplicates,
        "untimed_events": untimed_events,
        "earliest_time_us": earliest,
        "latest_time_us": latest,
        "source_size": snapshot_size,
        "hash_set_digest": digest.digest(),
    }


def reconcile(store: EventStore, root: Path, *, mark_ready: bool = False) -> dict:
    reports = []
    all_ready = True
    connection = store._connect(write=False)
    try:
        for install_id, path, relative in discover_sources(root):
            with tempfile.TemporaryDirectory(prefix="murmur-reconcile-") as directory:
                source = _scan_reconciliation_source(install_id, path, directory)
            row = connection.execute(
                """
                SELECT COUNT(*) AS count,
                       MIN(event_time_us) AS earliest,
                       MAX(event_time_us) AS latest,
                       SUM(CASE WHEN event_time_us IS NULL THEN 1 ELSE 0 END) AS untimed
                FROM events WHERE install_id = ?
                """,
                (install_id,),
            ).fetchone()
            database_digest = hashlib.sha256()
            for hash_row in connection.execute(
                "SELECT event_hash FROM events WHERE install_id = ? ORDER BY event_hash",
                (install_id,),
            ):
                database_digest.update(hash_row[0].encode("ascii"))
                database_digest.update(b"\n")
            checkpoint = connection.execute(
                """
                SELECT inserted_events, byte_offset, source_size, complete
                FROM backfill_checkpoints WHERE source_path = ?
                """,
                (relative,),
            ).fetchone()
            database_count = int(row["count"])
            expected_count = source["valid_objects"] - source["duplicates"]
            ready = (
                database_count == expected_count
                and int(row["untimed"] or 0) == source["untimed_events"]
                and row["earliest"] == source["earliest_time_us"]
                and row["latest"] == source["latest_time_us"]
                and checkpoint is not None
                and checkpoint["complete"] == 1
                and checkpoint["byte_offset"] == source["source_size"]
                and checkpoint["source_size"] == source["source_size"]
                and hmac.compare_digest(
                    database_digest.digest(), source["hash_set_digest"]
                )
            )
            all_ready = all_ready and ready
            reports.append(
                {
                    "install_id": install_id,
                    "raw_lines": source["raw_lines"],
                    "valid_objects": source["valid_objects"],
                    "malformed_lines": source["malformed_lines"],
                    "duplicates": source["duplicates"],
                    "untimed_events": source["untimed_events"],
                    "inserted_events": int(checkpoint[0]) if checkpoint else 0,
                    "database_count": database_count,
                    "earliest_timestamp": _format_timestamp(source["earliest_time_us"]),
                    "latest_timestamp": _format_timestamp(source["latest_time_us"]),
                    "ready": ready,
                }
            )
    except sqlite3.Error as error:
        raise _sqlite_error(error) from error
    finally:
        connection.close()
    if not reports:
        all_ready = False
    if mark_ready:
        if not all_ready:
            raise StoreError("reconciliation did not prove database readiness")
        store.set_dashboard_ready(True)
    return {
        "schema": "murmur-event-reconciliation/v1",
        "database_ready": all_ready,
        "dashboard_reads_enabled": store.is_dashboard_ready(),
        "installs": reports,
    }


def _format_timestamp(value: int | None) -> str | None:
    if value is None:
        return None
    return datetime.fromtimestamp(value / 1_000_000, timezone.utc).isoformat().replace(
        "+00:00", "Z"
    )


def _read_only_uri(path: str) -> str:
    return Path(path).as_uri() + "?mode=ro"


def restore_database(database: str, source: str) -> str:
    database = os.path.abspath(database)
    source = os.path.abspath(source)
    if database == source:
        raise StoreError("restore source must differ from database")
    validation = sqlite3.connect(_read_only_uri(source), uri=True)
    try:
        if validation.execute("PRAGMA integrity_check").fetchone()[0] != "ok":
            raise StoreCorrupt("restore source failed integrity check")
        version = validation.execute(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations"
        ).fetchone()[0]
        if version != SCHEMA_VERSION:
            raise StoreError("restore source schema is incompatible")
    finally:
        validation.close()
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S.%fZ")
    preserved = database + ".pre-restore-" + stamp
    temporary = database + ".restore-%d" % os.getpid()
    source_connection = sqlite3.connect(_read_only_uri(source), uri=True)
    temporary_connection = sqlite3.connect(temporary)
    try:
        source_connection.backup(temporary_connection, pages=256, sleep=0.05)
    finally:
        temporary_connection.close()
        source_connection.close()
    try:
        with open(temporary, "r+b") as handle:
            os.fsync(handle.fileno())
        if os.path.exists(database):
            for suffix in ("-wal", "-shm"):
                sidecar = database + suffix
                if os.path.exists(sidecar):
                    shutil.copy2(sidecar, preserved + suffix)
            current_connection = sqlite3.connect(_read_only_uri(database), uri=True)
            preserved_connection = sqlite3.connect(preserved)
            try:
                current_connection.backup(
                    preserved_connection, pages=256, sleep=0.05
                )
            except sqlite3.Error:
                preserved_connection.close()
                current_connection.close()
                try:
                    os.unlink(preserved)
                except FileNotFoundError:
                    pass
                shutil.copy2(database, preserved)
            else:
                preserved_connection.close()
                current_connection.close()
        os.replace(temporary, database)
        for suffix in ("-wal", "-shm"):
            try:
                os.unlink(database + suffix)
            except FileNotFoundError:
                pass
        _fsync_directory(os.path.dirname(database) or ".")
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise
    return preserved


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", default=os.path.expanduser("~/murmur-logs"))
    parser.add_argument("--database")
    parser.add_argument("--busy-timeout-ms", type=int, default=DEFAULT_BUSY_TIMEOUT_MS)
    parser.add_argument("--quota-bytes", type=int, default=DEFAULT_DATABASE_QUOTA_BYTES)
    commands = parser.add_subparsers(dest="command", required=True)
    backfill_parser = commands.add_parser("backfill")
    backfill_parser.add_argument("--max-lines", type=int, default=10_000)
    backfill_parser.add_argument("--batch-size", type=int, default=MAX_BACKFILL_BATCH)
    reconcile_parser = commands.add_parser("reconcile")
    reconcile_parser.add_argument("--mark-ready", action="store_true")
    commands.add_parser("integrity")
    backup_parser = commands.add_parser("backup")
    backup_parser.add_argument("destination")
    restore_parser = commands.add_parser("restore")
    restore_parser.add_argument("source")
    commands.add_parser("disable-dashboard")
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    root = Path(args.root).resolve()
    database = os.path.abspath(args.database or root / "events.sqlite3")
    if args.command == "restore":
        preserved = restore_database(database, args.source)
        print(json.dumps({"restored": True, "preserved_database": preserved}))
        return 0
    store = EventStore(
        database,
        busy_timeout_ms=args.busy_timeout_ms,
        quota_bytes=args.quota_bytes,
    )
    store.initialize()
    if args.command == "backfill":
        report = backfill(
            store, root, max_lines=args.max_lines, batch_size=args.batch_size
        )
    elif args.command == "reconcile":
        report = reconcile(store, root, mark_ready=args.mark_ready)
    elif args.command == "integrity":
        report = {
            "schema": "murmur-event-integrity/v1",
            "result": store.integrity_check(),
            "schema_version": store.schema_version(),
        }
    elif args.command == "backup":
        store.backup(args.destination)
        report = {"schema": "murmur-event-backup/v1", "complete": True}
    elif args.command == "disable-dashboard":
        store.set_dashboard_ready(False)
        report = {"dashboard_reads_enabled": False}
    else:
        raise AssertionError("unreachable command")
    print(json.dumps(report, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except StoreError as error:
        print("event store error: %s" % error, file=sys.stderr)
        raise SystemExit(1) from error
