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
    KEY_SCRUB_JOURNAL_NAME,
    RAW_COPY_BYTES,
    RAW_MAINTENANCE_LOCK_NAME,
    REDACTED_KEY,
    REDACTED_TEXT,
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
MAINTENANCE_LOCK_NAME = RAW_MAINTENANCE_LOCK_NAME
JOURNAL_NAME = KEY_SCRUB_JOURNAL_NAME
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
    '"name":',
    '"char":',
    '"character":',
    '"chars":',
    '"text":',
    '"unicode":',
    'key_name":',
    'key_text":',
)


LINE_MARKERS = tuple(marker.encode("ascii") for marker in ROW_MARKERS)
# Lines already in sanitized form: candidates for duplicate removal when an
# older line scrubs to the same event.
SANITIZED_MARKERS = (REDACTED_KEY.encode("ascii"), REDACTED_TEXT.encode("ascii"))


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


def _line_outcome(body: bytes, install_id: str | None) -> tuple[bytes, int, str | None]:
    """Scrubbed body, replaced-value count, and a dedupe hash when relevant.

    The hash is the event's install-scoped canonical hash, computed only for
    lines that changed or are already sanitized, and only when `install_id`
    is given (the production stream; the legacy dev lane is never deduped).
    """
    if not body.strip():
        return body, 0, None
    sanitized_marker = install_id is not None and any(
        marker in body for marker in SANITIZED_MARKERS
    )
    if not sanitized_marker and not any(marker in body for marker in LINE_MARKERS):
        return body, 0, None
    try:
        event = json.loads(body)
    except (UnicodeDecodeError, ValueError, RecursionError):
        return body, 0, None
    if not isinstance(event, dict):
        return body, 0, None
    sanitized, changed = sanitize_event(event)
    if not changed and not sanitized_marker:
        return body, 0, None
    canonical = _canonical(sanitized)
    replacement = canonical if changed else body
    digest = event_hash(install_id, canonical) if install_id is not None else None
    return replacement, changed, digest


class SourceScan:
    def __init__(self, wanted_offsets: set[int]) -> None:
        self.wanted = wanted_offsets
        self.offsets = {0: 0}
        self.changed_lines = 0
        self.redacted_values = 0
        self.dropped_lines = 0
        self.hashes: list[tuple[str, bool]] = []
        self.old_size = 0
        self.new_size = 0
        self.digest = hashlib.sha256()


def _scrubbed_chunks(
    source: RawSource,
    scan: SourceScan,
    install_id: str | None,
    duplicates: set[str],
    emitted: set[str],
):
    """Yield scrubbed bytes for one source while recording offsets/counts.

    A line whose hash is in `duplicates` is kept only on its first occurrence
    across the install's stream (`emitted` carries that state between
    sources); later copies are dropped.
    """
    pending = []
    pending_bytes = 0
    with open_raw_source(source) as handle:
        for line in iter(handle.readline, b""):
            scan.old_size += len(line)
            body = line.rstrip(b"\r\n")
            replacement, count, digest = _line_outcome(body, install_id)
            if count:
                scan.changed_lines += 1
                scan.redacted_values += count
                line = replacement + line[len(body):]
            keep = True
            if digest is not None:
                scan.hashes.append((digest, bool(count)))
                if digest in duplicates:
                    keep = digest not in emitted
                    emitted.add(digest)
            if keep:
                scan.new_size += len(line)
                scan.digest.update(line)
                pending.append(line)
                pending_bytes += len(line)
            else:
                scan.dropped_lines += 1
            if scan.old_size in scan.wanted:
                scan.offsets[scan.old_size] = scan.new_size
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
    install_id: str | None,
    duplicates: set[str],
    emitted: set[str],
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
                _scrubbed_chunks(source, scan, install_id, duplicates, emitted),
                directory,
                source.sequence,
            )
        else:
            temporary = os.path.join(
                directory,
                ".%s.scrub-%s.tmp"
                % (os.path.basename(source.path), secrets.token_hex(4)),
            )
            with open(temporary, "xb") as handle:
                for chunk in _scrubbed_chunks(
                    source, scan, install_id, duplicates, emitted
                ):
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


def _install_sources(directory: Path) -> list[RawSource]:
    sources = raw_sources(directory)
    dev = directory / "events.dev.jsonl"
    if dev.is_file():
        # Legacy dev lane: raw only, never projected or checkpointed.
        sources.append(RawSource(directory.name.lower(), 0, str(dev), False, False))
    return sources


def _may_contain_key(text: str) -> bool:
    return any(marker in text for marker in ROW_MARKERS)


def _empty_raw_counts(install_id: str) -> dict:
    return {
        "install_id": install_id,
        "raw_sources": 0,
        "raw_sources_rewritten": 0,
        "raw_lines_changed": 0,
        "raw_lines_deduplicated": 0,
        "raw_values_redacted": 0,
    }


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
                    # Rows that differed only by the typed key (or an old row
                    # and its sanitized retry) are now the same content-free
                    # event. Keep whichever row was stored first.
                    other = connection.execute(
                        "SELECT event_id FROM events WHERE install_id = ? AND event_hash = ?",
                        (install_id, new_hash),
                    ).fetchone()[0]
                    if other > last_id:
                        connection.execute("DELETE FROM events WHERE event_id = ?", (other,))
                        connection.execute(
                            "UPDATE events SET event_hash = ?, event_json = ?, summary = ? "
                            "WHERE event_id = ?",
                            (new_hash, canonical.decode("utf-8"), summary, last_id),
                        )
                    else:
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
        report = _empty_raw_counts(install_id)
        sources = _install_sources(directory)
        # Pass 1: count changes and collect hashes of changed or already
        # sanitized lines. A hash seen more than once with at least one
        # changed occurrence would leave duplicate raw lines after scrubbing.
        scans = []
        occurrences: dict[str, int] = {}
        changed_hashes: set[str] = set()
        for source in sources:
            scan = SourceScan(set())
            for _ in _scrubbed_chunks(
                source, scan, install_id if source.sequence else None, set(), set()
            ):
                pass
            for digest, changed in scan.hashes:
                occurrences[digest] = occurrences.get(digest, 0) + 1
                if changed:
                    changed_hashes.add(digest)
            scans.append(scan)
        duplicates = {digest for digest in changed_hashes if occurrences[digest] > 1}
        # Plan first occurrences in stream order, then rewrite affected sources.
        emitted: set[str] = set()
        plan = []
        for source, scan in zip(sources, scans):
            before = set(emitted)
            drops = 0
            if source.sequence:
                for digest, _ in scan.hashes:
                    if digest in duplicates:
                        drops += digest in emitted
                        emitted.add(digest)
            plan.append((source, scan, before, drops))
        connection = store._connect(write=False)
        try:
            for source, scan, before, drops in plan:
                report["raw_sources"] += 1
                if not scan.changed_lines and not drops:
                    continue
                report["raw_lines_changed"] += scan.changed_lines
                report["raw_values_redacted"] += scan.redacted_values
                report["raw_lines_deduplicated"] += drops
                if apply:
                    checkpoint = (
                        _checkpoint(connection, source.key) if source.sequence else None
                    )
                    rewritten = _rewrite_source(
                        root,
                        store,
                        source,
                        checkpoint,
                        install_id if source.sequence else None,
                        duplicates if source.sequence else set(),
                        before,
                    )
                    if (
                        rewritten.changed_lines != scan.changed_lines
                        or rewritten.dropped_lines != drops
                    ):
                        raise ScrubError("raw source changed during scrub")
                    report["raw_sources_rewritten"] += 1
        finally:
            connection.close()
        installs.append(report)
    database = scrub_database(store, apply=apply)
    known = {report["install_id"] for report in installs}
    for install_id in sorted(set(database) - known):
        installs.append(_empty_raw_counts(install_id))
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
