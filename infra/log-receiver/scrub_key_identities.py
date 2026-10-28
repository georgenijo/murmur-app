#!/usr/bin/env python3
"""One-shot scrub of typed-key identities from stored Murmur history.

App versions up to 0.52.2 shipped keyboard telemetry whose fields carried rdev
Debug forms such as `KeyPress(KeyI)` or `Some(Num4)`, naming the keys a user
typed. The receiver now sanitizes these at ingest (`event_store.sanitize_event`).
This tool applies the same rule to history stored before that change: every
raw JSONL source (live file, sealed gzip segments, legacy dev files) and the
SQLite projection, including its FTS index.

The default is `--dry-run`, which changes nothing and prints per-install
counts. `--apply` requires the receiver to be stopped, rewrites each affected
file through a fsynced temporary and an atomic rename, and updates SQLite in
one transaction with `secure_delete` on. No unscrubbed copy is
kept: a retained backup would retain the very key identities being removed.
Existing operator backups are out of scope and must be handled separately.
"""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import secrets
import sqlite3
import sys
from pathlib import Path

SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
if SCRIPT_DIR not in sys.path:
    sys.path.insert(0, SCRIPT_DIR)

from event_store import (  # noqa: E402
    INSTALL_ID_RE,
    RAW_COPY_BYTES,
    EventStore,
    RawSource,
    StoreError,
    _bounded_text,
    _fsync_directory,
    compact_pending_segments,
    event_hash,
    open_raw_source,
    raw_sources,
    sanitize_event,
    stage_gzip_segment,
)

REPORT_SCHEMA = "murmur-key-scrub/v1"
MAINTENANCE_LOCK_NAME = ".raw-maintenance.lock"
JOURNAL_NAME = ".key-scrub-journal.json"
DATABASE_PAGE_ROWS = 2_000
# Cheap prefilter before parsing a stored row; anything sanitize_event can
# change contains one of these substrings.
ROW_MARKERS = (
    "KeyPress(",
    "KeyRelease(",
    "Some(",
    '"key":',
    '_key":',
    '"event_type":',
    '"event_name":',
    '"reason":',
)


LINE_MARKERS = tuple(marker.encode("ascii") for marker in ROW_MARKERS)


class ScrubError(RuntimeError):
    pass


def _canonical(event: dict) -> bytes:
    # Same encoding as event_store.canonical_event, without its size bound:
    # an oversized line must still be scrubbed rather than kept verbatim.
    return json.dumps(
        event,
        ensure_ascii=False,
        allow_nan=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")


def scrub_line(body: bytes) -> tuple[bytes, int]:
    """Return the scrubbed line body and the number of replaced values."""
    if not body.strip() or not any(marker in body for marker in LINE_MARKERS):
        return body, 0
    try:
        event = json.loads(body)
    except (UnicodeDecodeError, ValueError, RecursionError):
        return body, 0
    if not isinstance(event, dict):
        return body, 0
    sanitized, changed = sanitize_event(event)
    if not changed:
        return body, 0
    return _canonical(sanitized), changed


class SourceScan:
    def __init__(self, wanted_offsets: set[int]) -> None:
        self.wanted = wanted_offsets
        self.offsets = {0: 0}
        self.changed_lines = 0
        self.redacted_values = 0
        self.old_size = 0
        self.new_size = 0
        self.digest = hashlib.sha256()


def _scrubbed_chunks(source: RawSource, scan: SourceScan):
    """Yield scrubbed bytes for one source while recording offsets/counts."""
    pending = []
    pending_bytes = 0
    with open_raw_source(source) as handle:
        for line in iter(handle.readline, b""):
            scan.old_size += len(line)
            body = line.rstrip(b"\r\n")
            replacement, count = scrub_line(body)
            if count:
                scan.changed_lines += 1
                scan.redacted_values += count
                line = replacement + line[len(body):]
            scan.new_size += len(line)
            scan.digest.update(line)
            if scan.old_size in scan.wanted:
                scan.offsets[scan.old_size] = scan.new_size
            pending.append(line)
            pending_bytes += len(line)
            if pending_bytes >= RAW_COPY_BYTES:
                yield b"".join(pending)
                pending = []
                pending_bytes = 0
    if pending:
        yield b"".join(pending)


def _uncompressed_digest(source: RawSource) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    with open_raw_source(source) as handle:
        for chunk in iter(lambda: handle.read(RAW_COPY_BYTES), b""):
            digest.update(chunk)
            size += len(chunk)
    return size, digest.hexdigest()


def _write_journal(root: Path, entry: dict) -> None:
    path = root / JOURNAL_NAME
    temporary = root / (JOURNAL_NAME + ".tmp")
    with open(temporary, "w", encoding="utf-8") as handle:
        json.dump(entry, handle, sort_keys=True)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temporary, path)
    _fsync_directory(str(root))


def _clear_journal(root: Path) -> None:
    try:
        os.unlink(root / JOURNAL_NAME)
    except FileNotFoundError:
        return
    _fsync_directory(str(root))


def _checkpoint(connection: sqlite3.Connection, key: str):
    return connection.execute(
        "SELECT byte_offset, source_size, complete FROM backfill_checkpoints "
        "WHERE source_path = ?",
        (key,),
    ).fetchone()


def _apply_checkpoint(store: EventStore, entry: dict) -> None:
    update = entry.get("checkpoint")
    if update is None:
        return
    connection = store._connect()
    try:
        connection.execute("BEGIN IMMEDIATE")
        connection.execute(
            "UPDATE backfill_checkpoints SET byte_offset = ?, source_size = ?, "
            "complete = ? WHERE source_path = ?",
            (
                update["byte_offset"],
                update["source_size"],
                update["complete"],
                entry["key"],
            ),
        )
        connection.execute("COMMIT")
    except BaseException:
        try:
            connection.execute("ROLLBACK")
        except sqlite3.Error:
            pass
        raise
    finally:
        connection.close()


def recover_journal(root: Path, store: EventStore) -> None:
    """Finish or discard a file rewrite interrupted before its checkpoint."""
    path = root / JOURNAL_NAME
    try:
        entry = json.loads(path.read_text(encoding="utf-8"))
    except FileNotFoundError:
        return
    source = RawSource(
        entry["install_id"],
        entry["sequence"],
        entry["path"],
        entry["compressed"],
        entry["sealed"],
    )
    current = _uncompressed_digest(source)
    if current == (entry["new_size"], entry["new_sha256"]):
        _apply_checkpoint(store, entry)
    elif current[0] != entry["old_size"]:
        raise ScrubError("journaled raw source matches neither side of its rewrite")
    _clear_journal(root)


def _rewrite_source(
    root: Path,
    store: EventStore,
    source: RawSource,
    checkpoint,
) -> SourceScan:
    """Stage a scrubbed copy, journal it, then atomically replace the source."""
    wanted = set()
    if checkpoint is not None:
        wanted = {int(checkpoint["byte_offset"]), int(checkpoint["source_size"])}
    scan = SourceScan(wanted)
    directory = os.path.dirname(source.path)
    temporary = None
    try:
        if source.compressed:
            temporary, _ = stage_gzip_segment(
                _scrubbed_chunks(source, scan), directory, source.sequence
            )
        else:
            temporary = os.path.join(
                directory,
                ".%s.scrub-%s.tmp"
                % (os.path.basename(source.path), secrets.token_hex(4)),
            )
            with open(temporary, "xb") as handle:
                for chunk in _scrubbed_chunks(source, scan):
                    handle.write(chunk)
                handle.flush()
                os.fsync(handle.fileno())
        entry = {
            "install_id": source.install_id,
            "sequence": source.sequence,
            "path": source.path,
            "compressed": source.compressed,
            "sealed": source.sealed,
            "key": source.key,
            "old_size": scan.old_size,
            "new_size": scan.new_size,
            "new_sha256": scan.digest.hexdigest(),
            "checkpoint": None,
        }
        if checkpoint is not None:
            offset = int(checkpoint["byte_offset"])
            source_size = int(checkpoint["source_size"])
            if offset in scan.offsets and source_size in scan.offsets:
                entry["checkpoint"] = {
                    "byte_offset": scan.offsets[offset],
                    "source_size": scan.offsets[source_size],
                    "complete": int(checkpoint["complete"]),
                }
            else:
                # Not on a line boundary; restart this source's idempotent backfill.
                entry["checkpoint"] = {"byte_offset": 0, "source_size": 0, "complete": 0}
        # Journal before the rename so an interrupted run can tell whether the
        # file was replaced and finish the matching checkpoint update.
        _write_journal(root, entry)
        os.replace(temporary, source.path)
        temporary = None
        _fsync_directory(directory)
        _apply_checkpoint(store, entry)
        _clear_journal(root)
    finally:
        if temporary is not None:
            try:
                os.unlink(temporary)
            except FileNotFoundError:
                pass
    return scan


def _count_source(source: RawSource) -> SourceScan:
    scan = SourceScan(set())
    for _ in _scrubbed_chunks(source, scan):
        pass
    return scan


def _install_sources(directory: Path) -> list[RawSource]:
    sources = raw_sources(directory)
    dev = directory / "events.dev.jsonl"
    if dev.is_file():
        # Legacy dev lane: raw only, never projected or checkpointed.
        sources.append(RawSource(directory.name.lower(), 0, str(dev), False, False))
    return sources


def _may_contain_key(text: str) -> bool:
    return any(marker in text for marker in ROW_MARKERS)


def _empty_database_counts() -> dict:
    return {
        "database_rows_changed": 0,
        "database_rows_merged": 0,
        "database_summaries_changed": 0,
    }


def scrub_database(store: EventStore, *, apply: bool) -> dict[str, dict]:
    """Scrub every stored row in one rowid-ordered pass.

    Returns per-install counts. With `apply`, all changes commit in a single
    transaction with `secure_delete` on so freed cells are overwritten.
    """
    connection = store._connect(write=apply)
    counts: dict[str, dict] = {}
    seen_hashes: set[tuple[str, str]] = set()
    summaries = 0
    try:
        if apply:
            connection.execute("PRAGMA secure_delete = ON")
            connection.execute("BEGIN IMMEDIATE")
        last_id = 0
        while True:
            # Primary-key range scan: per-install index filters would re-sort
            # the whole install for every page.
            rows = connection.execute(
                "SELECT event_id, install_id, event_json, summary FROM events "
                "WHERE event_id > ? ORDER BY event_id LIMIT ?",
                (last_id, DATABASE_PAGE_ROWS),
            ).fetchall()
            if not rows:
                break
            for row in rows:
                last_id = int(row["event_id"])
                if not _may_contain_key(row["event_json"]):
                    continue
                try:
                    event = json.loads(row["event_json"])
                except (ValueError, RecursionError):
                    continue
                if not isinstance(event, dict):
                    continue
                sanitized, count = sanitize_event(event)
                if not count:
                    continue
                install_id = row["install_id"]
                install = counts.setdefault(install_id, _empty_database_counts())
                canonical = _canonical(sanitized)
                new_hash = event_hash(install_id, canonical)
                summary = _bounded_text(sanitized.get("summary"), 2048)
                install["database_rows_changed"] += 1
                if summary != row["summary"]:
                    install["database_summaries_changed"] += 1
                    summaries += 1
                if not apply:
                    collides = (install_id, new_hash) in seen_hashes or connection.execute(
                        "SELECT 1 FROM events WHERE install_id = ? AND event_hash = ?",
                        (install_id, new_hash),
                    ).fetchone() is not None
                    seen_hashes.add((install_id, new_hash))
                    install["database_rows_merged"] += int(collides)
                    continue
                try:
                    connection.execute(
                        "UPDATE events SET event_hash = ?, event_json = ?, summary = ? "
                        "WHERE event_id = ?",
                        (new_hash, canonical.decode("utf-8"), summary, last_id),
                    )
                except sqlite3.IntegrityError:
                    # Two old rows differed only by the typed key; they are
                    # now the same content-free event. Keep the earlier row.
                    connection.execute(
                        "DELETE FROM events WHERE event_id = ?", (last_id,)
                    )
                    install["database_rows_merged"] += 1
        if apply:
            if summaries:
                connection.execute(
                    "INSERT INTO events_fts(events_fts) VALUES ('optimize')"
                )
            connection.execute("COMMIT")
    except BaseException:
        if apply:
            try:
                connection.execute("ROLLBACK")
            except sqlite3.Error:
                pass
        raise
    finally:
        connection.close()
    return counts


def scrub(root: Path, store: EventStore, *, apply: bool) -> dict:
    installs = []
    if apply:
        recover_journal(root, store)
    for directory in sorted(root.iterdir()) if root.is_dir() else []:
        if not directory.is_dir() or not INSTALL_ID_RE.fullmatch(directory.name):
            continue
        install_id = directory.name.lower()
        if apply:
            for stale in directory.glob(".*.scrub-*.tmp"):
                stale.unlink()
            compact_pending_segments(directory)
        report = {
            "install_id": install_id,
            "raw_sources": 0,
            "raw_sources_rewritten": 0,
            "raw_lines_changed": 0,
            "raw_values_redacted": 0,
        }
        connection = store._connect(write=False)
        try:
            for source in _install_sources(directory):
                report["raw_sources"] += 1
                scan = _count_source(source)
                if not scan.changed_lines:
                    continue
                report["raw_lines_changed"] += scan.changed_lines
                report["raw_values_redacted"] += scan.redacted_values
                if apply:
                    checkpoint = (
                        _checkpoint(connection, source.key) if source.sequence else None
                    )
                    rewritten = _rewrite_source(root, store, source, checkpoint)
                    if rewritten.changed_lines != scan.changed_lines:
                        raise ScrubError("raw source changed during scrub")
                    report["raw_sources_rewritten"] += 1
        finally:
            connection.close()
        installs.append(report)
    database = scrub_database(store, apply=apply)
    known = {report["install_id"] for report in installs}
    for install_id in sorted(set(database) - known):
        installs.append(
            {
                "install_id": install_id,
                "raw_sources": 0,
                "raw_sources_rewritten": 0,
                "raw_lines_changed": 0,
                "raw_values_redacted": 0,
            }
        )
    for report in installs:
        report.update(database.get(report["install_id"], _empty_database_counts()))
    if apply:
        connection = store._connect()
        try:
            connection.execute("PRAGMA wal_checkpoint(TRUNCATE)")
        finally:
            connection.close()
    totals = {}
    for report in installs:
        for key, value in report.items():
            if key != "install_id":
                totals[key] = totals.get(key, 0) + value
    return {
        "schema": REPORT_SCHEMA,
        "mode": "apply" if apply else "dry-run",
        "installs": installs,
        "totals": totals,
    }


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("--root", default=os.path.expanduser("~/murmur-logs"))
    parser.add_argument("--database")
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--dry-run",
        action="store_true",
        help="report per-install counts without changing anything (default)",
    )
    mode.add_argument(
        "--apply",
        action="store_true",
        help="rewrite raw files and SQLite in place; keeps no unscrubbed backup",
    )
    return parser


def main(argv=None) -> int:
    args = _parser().parse_args(argv)
    root = Path(args.root).resolve()
    database = os.path.abspath(args.database or root / "events.sqlite3")
    if not os.path.exists(database):
        print("event store not found: %s" % database, file=sys.stderr)
        return 1
    store = EventStore(database)
    lock = None
    if args.apply:
        lock = open(root / MAINTENANCE_LOCK_NAME, "a+b")
        try:
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            print(
                "receiver is running; stop murmur-logs.service before --apply",
                file=sys.stderr,
            )
            return 2
    try:
        if args.apply:
            store.initialize()
        # A dry run only reads: no migration, lock, compaction, or journal.
        report = scrub(root, store, apply=args.apply)
    finally:
        if lock is not None:
            lock.close()
    print(json.dumps(report, sort_keys=True, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (ScrubError, StoreError, OSError) as error:
        print("key scrub failed: %s" % type(error).__name__, file=sys.stderr)
        raise SystemExit(1) from error
