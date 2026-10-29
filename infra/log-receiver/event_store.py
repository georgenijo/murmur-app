#!/usr/bin/env python3
"""Bounded SQLite query store for the Murmur diagnostic-log receiver.

The raw per-install JSONL stream remains the recovery and export source. It is
the ordered concatenation of immutable sealed segments under
``<install>/sealed/`` followed by the live ``<install>/events.jsonl``. This
module maintains an idempotent indexed projection, resumable backfill
checkpoints, and a persisted readiness gate for historical dashboard reads.
It intentionally uses only the Python standard library.
"""

from __future__ import annotations

import argparse
import base64
import gzip
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
RAW_LIVE_NAME = "events.jsonl"
# Deliberately not "archive": older installs already keep hang bundles there.
SEALED_DIRECTORY = "sealed"
SEALED_PENDING_RE = re.compile(r"^events-(\d{6})\.pending\.jsonl$")
SEALED_SEGMENT_RE = re.compile(
    r"^events-(\d{6})-(\d{8}T\d{6}Z|untimed)-(\d{8}T\d{6}Z|untimed)\.jsonl\.gz$"
)
SEALED_TEMPORARY_RE = re.compile(r"^\.events-\d{6}\..*\.tmp$")
SEAL_GZIP_LEVEL = 6
RAW_COPY_BYTES = 1024 * 1024
# Held shared by the running receiver and exclusively by the scrub tool.
RAW_MAINTENANCE_LOCK_NAME = ".raw-maintenance.lock"
# Present only while a scrub rewrite is unfinished; the receiver refuses to
# start until a rerun of the scrub completes it.
KEY_SCRUB_JOURNAL_NAME = ".key-scrub-journal.json"

# rdev `Key` variants. Old clients (<= 0.52.2) shipped their Debug form, which
# names the physical key the user typed. Only configured hotkeys may remain.
RDEV_KEYS = frozenset(
    """
    Alt AltGr Backspace CapsLock ControlLeft ControlRight Delete DownArrow End
    Escape F1 F2 F3 F4 F5 F6 F7 F8 F9 F10 F11 F12 F13 F14 F15 F16 F17 F18 F19
    F20 F21 F22 F23 F24 Home LeftArrow MetaLeft MetaRight PageDown PageUp Return
    RightArrow ShiftLeft ShiftRight Space Tab UpArrow PrintScreen ScrollLock
    Pause NumLock BackQuote Num1 Num2 Num3 Num4 Num5 Num6 Num7 Num8 Num9 Num0
    Minus Equal KeyQ KeyW KeyE KeyR KeyT KeyY KeyU KeyI KeyO KeyP LeftBracket
    RightBracket KeyA KeyS KeyD KeyF KeyG KeyH KeyJ KeyK KeyL SemiColon Quote
    BackSlash IntlBackslash KeyZ KeyX KeyC KeyV KeyB KeyN KeyM Comma Dot Slash
    Insert KpReturn KpMinus KpPlus KpMultiply KpDivide Kp0 Kp1 Kp2 Kp3 Kp4 Kp5
    Kp6 Kp7 Kp8 Kp9 KpDelete Function VolumeUp VolumeDown VolumeMute
    BrightnessUp BrightnessDown PreviousTrack PlayPause PlayCd NextTrack
    """.split()
)
HOTKEY_KEYS = frozenset(
    (
        "Alt",
        "AltGr",
        "ControlLeft",
        "ControlRight",
        "MetaLeft",
        "MetaRight",
        "ShiftLeft",
        "ShiftRight",
        "Function",
        *("F%d" % number for number in range(1, 21)),
    )
)
REDACTED_KEY = "non_hotkey_key"
REDACTED_TEXT = "redacted_text"
_KEY_NAME_PATTERN = "(?:%s|Unknown\\(\\d+\\))" % "|".join(
    sorted(RDEV_KEYS, key=len, reverse=True)
)
# Token boundaries keep paths and identifiers such as `/models/KeyA/`,
# `KeyA-7B`, or `MyKeyPress(KeyA)` untouched.
_EMBEDDED_KEY_EVENT_RE = re.compile(
    r"(?<![A-Za-z0-9_])(KeyPress|KeyRelease)\((%s)\)(?![A-Za-z0-9_])" % _KEY_NAME_PATTERN
)
_EMBEDDED_SOME_KEY_RE = re.compile(
    r"(?<![A-Za-z0-9_])Some\((%s)\)(?![A-Za-z0-9_])" % _KEY_NAME_PATTERN
)
_BARE_KEY_RE = re.compile(r"^%s$" % _KEY_NAME_PATTERN)
_SOME_QUOTED_RE = re.compile(r'^Some\("(?:[^"\\]|\\.)*"\)$')
_SOME_ANY_RE = re.compile(r"^Some\(.*\)$", re.DOTALL)
# Fields that carry the character a key produced. `event_name` is rdev's
# `Event::name` everywhere; the others only inside keyboard-stream events.
_TEXT_FIELDS_ANYWHERE = frozenset(("event_name",))
_TEXT_FIELDS_IN_KEYBOARD = frozenset(
    ("name", "char", "character", "chars", "text", "unicode", "key_name", "key_text")
)
_ALREADY_REDACTED = frozenset(
    ("", "None", REDACTED_TEXT, "Some(%s)" % REDACTED_TEXT, REDACTED_KEY)
)
MAX_SANITIZE_DEPTH = 64


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
    sealed: bool = False


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


def _is_key_field(name: str) -> bool:
    return name in ("key", "event_type", "event_name", "reason") or name.endswith("_key")


def _redact_key_event(match: re.Match) -> str:
    if match.group(2) in HOTKEY_KEYS:
        return match.group(0)
    return "%s(%s)" % (match.group(1), REDACTED_KEY)


def _redact_some_key(match: re.Match) -> str:
    if match.group(1) in HOTKEY_KEYS:
        return match.group(0)
    return "Some(%s)" % REDACTED_KEY


def _sanitize_key_text(value: str, field: str, keyboard_event: bool) -> tuple[str, int]:
    if field in _TEXT_FIELDS_ANYWHERE or (
        keyboard_event and field in _TEXT_FIELDS_IN_KEYBOARD
    ):
        if value in _ALREADY_REDACTED:
            return value, 0
        if _SOME_ANY_RE.fullmatch(value):
            return "Some(%s)" % REDACTED_TEXT, 1
        return REDACTED_TEXT, 1
    sanitized = _EMBEDDED_KEY_EVENT_RE.sub(_redact_key_event, value)
    sanitized = _EMBEDDED_SOME_KEY_RE.sub(_redact_some_key, sanitized)
    changed = sanitized != value
    if _is_key_field(field):
        if _SOME_QUOTED_RE.fullmatch(sanitized):
            return "Some(%s)" % REDACTED_TEXT, 1
        if _BARE_KEY_RE.fullmatch(sanitized) and sanitized not in HOTKEY_KEYS:
            return REDACTED_KEY, 1
    return sanitized, int(changed)


def _sanitize_value(
    value: object, field: str, keyboard_event: bool, depth: int
) -> tuple[object, int]:
    if depth > MAX_SANITIZE_DEPTH:
        # Producers emit at most a few levels; never reclassify stored history.
        return value, 0
    if isinstance(value, str):
        return _sanitize_key_text(value, field, keyboard_event)
    if isinstance(value, dict):
        changed = 0
        result = {}
        for name, item in value.items():
            result[name], count = _sanitize_value(item, name, keyboard_event, depth + 1)
            changed += count
        return (result if changed else value), changed
    if isinstance(value, list):
        changed = 0
        result = []
        for item in value:
            sanitized, count = _sanitize_value(item, field, keyboard_event, depth + 1)
            result.append(sanitized)
            changed += count
        return (result if changed else value), changed
    return value, 0


def sanitize_event(event: dict) -> tuple[dict, int]:
    """Replace typed-key identities from old clients with a content-free class.

    rdev Debug forms embedded anywhere in any string (`KeyPress(KeyI)`,
    `KeyRelease(Num4)`, `Some(Slash)`, including summaries) become
    `non_hotkey_key`. In key fields a bare key name or `Some("a")` is
    replaced too, and character-bearing name fields (`event_name`, plus
    `name`/`char`/`text`-style fields of keyboard events) become
    `redacted_text`. Configured hotkeys (modifiers, Function, F1-F20) and
    unrelated values such as `Some(Idle)` detector states are kept. Returns
    the event and the number of changed values; an unchanged event is
    returned as-is.
    """
    if not isinstance(event, dict):
        raise InvalidEvent("event must be an object")
    keyboard_event = event.get("stream") == "keyboard" or event.get("target") == "keyboard"
    sanitized, changed = _sanitize_value(event, "", keyboard_event, 0)
    return sanitized, changed


def prepare_event(install_id: str, event: dict) -> PreparedEvent:
    event, _ = sanitize_event(event)
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


@dataclass(frozen=True)
class RawSource:
    """One ordered part of an install's raw stream.

    `sequence` is 1-based. A live file becomes the sealed segment with the same
    sequence, so its backfill checkpoint key never changes when it is sealed.
    """

    install_id: str
    sequence: int
    path: str
    compressed: bool
    sealed: bool

    @property
    def key(self) -> str:
        return raw_source_key(self.install_id, self.sequence)


def raw_source_key(install_id: str, sequence: int) -> str:
    base = "%s/%s" % (install_id, RAW_LIVE_NAME)
    return base if sequence == 1 else "%s#%d" % (base, sequence)


def raw_sources(install_directory: str | os.PathLike[str]) -> list[RawSource]:
    """Return sealed segments in order followed by the live file, if any.

    A published gzip segment wins over a pending (renamed but not yet
    compressed) file with the same sequence; both hold identical bytes.
    Temporary files are never sources. Gaps fail closed.
    """
    directory = os.fspath(install_directory)
    install_id = normalize_install_id(os.path.basename(os.path.normpath(directory)))
    sealed_directory = os.path.join(directory, SEALED_DIRECTORY)
    segments: dict[int, RawSource] = {}
    try:
        names = os.listdir(sealed_directory)
    except FileNotFoundError:
        names = []
    for name in names:
        match = SEALED_SEGMENT_RE.fullmatch(name)
        compressed = match is not None
        if match is None:
            match = SEALED_PENDING_RE.fullmatch(name)
        if match is None:
            continue
        sequence = int(match.group(1))
        existing = segments.get(sequence)
        if compressed and existing is not None and existing.compressed:
            raise StoreCorrupt("duplicate sealed raw segment")
        if compressed or existing is None:
            segments[sequence] = RawSource(
                install_id,
                sequence,
                os.path.join(sealed_directory, name),
                compressed,
                True,
            )
    if sorted(segments) != list(range(1, len(segments) + 1)):
        raise StoreCorrupt("sealed raw segments are not contiguous")
    sources = [segments[sequence] for sequence in sorted(segments)]
    live = os.path.join(directory, RAW_LIVE_NAME)
    if os.path.isfile(live):
        sources.append(RawSource(install_id, len(segments) + 1, live, False, False))
    return sources


def _current_source(source: RawSource) -> RawSource:
    """Re-resolve a pending segment that finished compaction after listing."""
    if not source.sealed or source.compressed or os.path.exists(source.path):
        return source
    directory = os.path.dirname(os.path.dirname(source.path))
    for candidate in raw_sources(directory):
        if candidate.sequence == source.sequence and candidate.sealed:
            return candidate
    raise FileNotFoundError(source.path)


def _gzip_length(raw_handle) -> int:
    """gzip ISIZE of an opened file (segments are far below 4 GiB)."""
    descriptor = raw_handle.fileno()
    size = os.fstat(descriptor).st_size
    if size < 18:
        raise StoreCorrupt("sealed raw segment is truncated")
    return int.from_bytes(os.pread(descriptor, 4, size - 4), "little")


def open_pinned_source(source: RawSource) -> tuple[RawSource, object, int]:
    """Open a source, returning the resolved source, handle, and length.

    The kind (gzip or plain) and the uncompressed length both come from the
    inode actually opened, so a compaction or rename racing this call cannot
    pair a gzip stream with a plain length. A pending segment that finished
    compaction before it could be opened is re-resolved to its gzip.
    """
    for _ in range(3):
        source = _current_source(source)
        try:
            if source.compressed:
                handle = gzip.open(source.path, "rb")
                try:
                    return source, handle, _gzip_length(handle.fileobj)
                except BaseException:
                    handle.close()
                    raise
            handle = open(source.path, "rb")
            return source, handle, os.fstat(handle.fileno()).st_size
        except FileNotFoundError:
            if not source.sealed or source.compressed:
                raise
    raise FileNotFoundError(source.path)


def open_raw_source(source: RawSource):
    return open_pinned_source(source)[1]


def raw_source_size(source: RawSource) -> int:
    """Uncompressed byte length of a source, measured on one opened inode."""
    _, handle, size = open_pinned_source(source)
    handle.close()
    return size


def _handle_identity(handle) -> tuple[int, int, int]:
    raw = handle.fileobj if isinstance(handle, gzip.GzipFile) else handle
    status = os.fstat(raw.fileno())
    return status.st_dev, status.st_ino, status.st_mtime_ns


def open_raw_snapshot(
    install_directory: str | os.PathLike[str], *, attempts: int = 5
) -> list[tuple[RawSource, object, int]]:
    """Open every raw source with lengths fixed at open time.

    Open descriptors pin inodes, so later appends, seals, or compaction cannot
    change what is read. The snapshot is accepted only if, after opening, the
    sealed count is unchanged and the live path still names the opened inode;
    otherwise a seal raced the open and it is retried. Callers must close
    every handle.
    """
    directory = os.fspath(install_directory)
    for _ in range(attempts):
        sources = raw_sources(directory)
        opened: list[tuple[RawSource, object, int]] = []
        try:
            for source in sources:
                opened.append(open_pinned_source(source))
            stable = sum(1 for source in sources if source.sealed) == sum(
                1 for source in raw_sources(directory) if source.sealed
            )
            live = [item for item in opened if not item[0].sealed]
            if stable and live:
                try:
                    current = os.stat(live[0][0].path)
                    identity = _handle_identity(live[0][1])
                    stable = (current.st_dev, current.st_ino) == identity[:2]
                except FileNotFoundError:
                    stable = False
            if stable:
                return opened
        except FileNotFoundError:
            pass
        except BaseException:
            for _, handle, _ in opened:
                handle.close()
            raise
        for _, handle, _ in opened:
            handle.close()
    raise StoreBusy("raw sources changed while opening")


def close_snapshot(snapshot: Sequence[tuple[RawSource, object, int]]) -> None:
    for _, handle, _ in snapshot:
        handle.close()


def iter_snapshot_lines(snapshot: Sequence[tuple[RawSource, object, int]]):
    """Yield `(raw_line, oversized)` across an opened snapshot, in order."""
    for _, handle, size in snapshot:
        # Handles are shared by every pass over a snapshot; always start at 0.
        handle.seek(0)
        offset = 0
        while offset < size:
            raw, consumed, oversized = _bounded_line(handle, size - offset)
            if consumed <= 0:
                break
            offset += consumed
            yield raw, oversized


def raw_source_disk_bytes(sources: Sequence[RawSource]) -> int:
    total = 0
    for source in sources:
        try:
            total += os.path.getsize(_current_source(source).path)
        except OSError:
            pass
    return total


def seal_live_file(install_directory: str | os.PathLike[str]) -> RawSource | None:
    """Atomically move a non-empty live file into the sealed sequence.

    Callers must exclude concurrent appends (ingest does this inside its
    SQLite write transaction). The rename is the commit point: before it the
    bytes are live, after it they are sealed segment N, and the next live file
    is N + 1. Compression happens later in `compact_pending_segments`.
    """
    directory = os.fspath(install_directory)
    install_id = normalize_install_id(os.path.basename(os.path.normpath(directory)))
    live = os.path.join(directory, RAW_LIVE_NAME)
    try:
        if os.path.getsize(live) == 0:
            return None
    except FileNotFoundError:
        return None
    sources = raw_sources(directory)
    sequence = sum(1 for source in sources if source.sealed) + 1
    sealed_directory = os.path.join(directory, SEALED_DIRECTORY)
    pending = os.path.join(sealed_directory, "events-%06d.pending.jsonl" % sequence)
    try:
        if not os.path.isdir(sealed_directory):
            os.mkdir(sealed_directory)
            _fsync_directory(directory)
        if os.path.lexists(pending):
            raise StoreCorrupt("sealed raw segment already exists")
        os.rename(live, pending)
        _fsync_directory(sealed_directory)
        _fsync_directory(directory)
    except OSError as error:
        raise ArchiveError("raw segment seal failed") from error
    return RawSource(install_id, sequence, pending, False, True)


def _segment_time(raw: bytes) -> str:
    try:
        value = json.loads(raw)
    except (UnicodeDecodeError, ValueError, RecursionError):
        return "untimed"
    value_us = timestamp_us(value.get("timestamp")) if isinstance(value, dict) else None
    if value_us is None:
        return "untimed"
    return datetime.fromtimestamp(value_us / 1_000_000, timezone.utc).strftime(
        "%Y%m%dT%H%M%SZ"
    )


def _segment_bounds(path: str) -> tuple[str, str]:
    """Timestamps of the first and last lines, used only in the segment name."""
    with open(path, "rb") as handle:
        first, _, _ = _bounded_line(handle, os.path.getsize(path))
        size = handle.seek(0, os.SEEK_END)
        handle.seek(max(0, size - MAX_EVENT_BYTES - 2))
        tail = handle.read().rstrip(b"\r\n")
    last = tail.rsplit(b"\n", 1)[-1]
    return _segment_time(first.strip()), _segment_time(last.strip())


def _write_gzip(chunks, temporary: str) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    with open(temporary, "xb") as raw_handle:
        with gzip.GzipFile(
            filename="",
            mode="wb",
            fileobj=raw_handle,
            compresslevel=SEAL_GZIP_LEVEL,
            mtime=0,
        ) as compressed:
            for chunk in chunks:
                compressed.write(chunk)
                digest.update(chunk)
                size += len(chunk)
        raw_handle.flush()
        os.fsync(raw_handle.fileno())
    return size, digest.hexdigest()


def _file_chunks(handle):
    while True:
        chunk = handle.read(RAW_COPY_BYTES)
        if not chunk:
            return
        yield chunk


def _digest_stream(handle) -> tuple[int, str]:
    digest = hashlib.sha256()
    size = 0
    for chunk in _file_chunks(handle):
        digest.update(chunk)
        size += len(chunk)
    return size, digest.hexdigest()


def _digest_gzip(path: str) -> tuple[int, str]:
    try:
        with gzip.open(path, "rb") as handle:
            return _digest_stream(handle)
    except (OSError, EOFError, gzip.BadGzipFile) as error:
        raise StoreCorrupt("sealed raw segment is unreadable") from error


def stage_gzip_segment(
    chunks, directory: str, sequence: int
) -> tuple[str, tuple[int, str]]:
    """Write and verify a fsynced gzip temporary; the caller publishes it.

    Returns the temporary path and the uncompressed (size, sha256). The name
    matches SEALED_TEMPORARY_RE, so compaction removes it after a crash.
    """
    temporary = os.path.join(
        directory, ".events-%06d.%s.tmp" % (sequence, secrets.token_hex(4))
    )
    try:
        written = _write_gzip(chunks, temporary)
        if _digest_gzip(temporary) != written:
            raise ArchiveError("sealed raw segment verification failed")
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise
    return temporary, written


def write_sealed_segment(chunks, final_path: str) -> tuple[int, str]:
    """Write, fsync, verify, and atomically publish one gzip segment."""
    directory = os.path.dirname(final_path)
    match = SEALED_SEGMENT_RE.fullmatch(os.path.basename(final_path))
    if match is None:
        raise StoreError("invalid sealed segment name")
    temporary, written = stage_gzip_segment(chunks, directory, int(match.group(1)))
    try:
        os.replace(temporary, final_path)
        _fsync_directory(directory)
    except BaseException:
        try:
            os.unlink(temporary)
        except FileNotFoundError:
            pass
        raise
    return written


def compact_pending_segments(install_directory: str | os.PathLike[str]) -> int:
    """Compress pending sealed segments; safe to rerun after any crash.

    Each step leaves a valid source set: the pending file stays authoritative
    until a verified gzip with identical bytes is published, and only then is
    the pending file removed. Callers serialize compaction per process.
    """
    directory = os.fspath(install_directory)
    sealed_directory = os.path.join(directory, SEALED_DIRECTORY)
    if not os.path.isdir(sealed_directory):
        return 0
    names = os.listdir(sealed_directory)
    for name in names:
        if SEALED_TEMPORARY_RE.fullmatch(name):
            os.unlink(os.path.join(sealed_directory, name))
    published = {}
    for name in names:
        match = SEALED_SEGMENT_RE.fullmatch(name)
        if match:
            published[int(match.group(1))] = os.path.join(sealed_directory, name)
    compacted = 0
    for name in sorted(names):
        match = SEALED_PENDING_RE.fullmatch(name)
        if match is None:
            continue
        sequence = int(match.group(1))
        pending = os.path.join(sealed_directory, name)
        if sequence in published:
            with open(pending, "rb") as handle:
                expected = _digest_stream(handle)
            if _digest_gzip(published[sequence]) != expected:
                raise StoreCorrupt("sealed raw segment differs from its pending source")
        else:
            first, last = _segment_bounds(pending)
            final = os.path.join(
                sealed_directory,
                "events-%06d-%s-%s.jsonl.gz" % (sequence, first, last),
            )
            with open(pending, "rb") as handle:
                write_sealed_segment(_file_chunks(handle), final)
        os.unlink(pending)
        _fsync_directory(sealed_directory)
        compacted += 1
    return compacted


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
            # Overview aggregates must not fetch every event's JSON-bearing
            # table row. Install this for existing v1 databases as well.
            connection.execute(
                "CREATE INDEX IF NOT EXISTS events_install_overview "
                "ON events(install_id, event_time_us, received_at_us)"
            )
            # Fleet-wide history has no install/level/stream/version prefix.
            connection.execute(
                "CREATE INDEX IF NOT EXISTS events_global_time "
                "ON events(COALESCE(event_time_us, received_at_us) DESC, event_id DESC)"
            )
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

    @staticmethod
    def _seal_if_full(
        path: str, lines: Sequence[bytes], max_bytes: int | None
    ) -> bool:
        if not lines or max_bytes is None:
            return False
        try:
            original_size = os.path.getsize(path)
        except FileNotFoundError:
            return False
        except OSError as error:
            raise ArchiveError("raw archive size is unavailable") from error
        append_bytes = sum(len(line) + 1 for line in lines)
        if original_size == 0 or original_size + append_bytes <= max_bytes:
            return False
        if append_bytes > max_bytes:
            return False
        return seal_live_file(os.path.dirname(path)) is not None

    def ingest_batch(
        self,
        install_id: str,
        events: Sequence[dict],
        *,
        metadata: dict | None,
        archive_path: str,
        archive_quota_bytes: int | None = None,
        seal_full_archive: bool = False,
        received_at: float | None = None,
    ) -> IngestResult:
        """Insert new events and append their canonical lines in one unit.

        With `seal_full_archive`, a live file that would exceed
        `archive_quota_bytes` is sealed first and the batch starts a fresh live
        file. The seal is kept even if this batch later rolls back: it moves
        bytes without changing the raw stream.
        """
        install_id = normalize_install_id(install_id)
        if not events:
            raise InvalidEvent("empty batch")
        prepared = [prepare_event(install_id, event) for event in events]
        received_at_us = int(
            (time.time() if received_at is None else received_at) * 1_000_000
        )
        connection = self._connect()
        archive_checkpoint = None
        sealed = False
        inserted_lines: list[bytes] = []
        try:
            connection.execute("BEGIN IMMEDIATE")
            self._upsert_install(connection, install_id, metadata, received_at_us)
            for item in prepared:
                if self._insert_prepared(connection, install_id, item, received_at_us):
                    inserted_lines.append(item.canonical)
            if self._logical_database_bytes(connection) > self.quota_bytes:
                raise StoreQuota("database quota exceeded")
            if seal_full_archive:
                # The open write transaction serializes this with every other
                # append, so no line can land in the file while it is renamed.
                sealed = self._seal_if_full(
                    archive_path, inserted_lines, archive_quota_bytes
                )
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
        return IngestResult(len(events), inserted, len(events) - inserted, sealed)

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


def iter_source_lines(source: RawSource):
    """Yield `(raw_line, oversized)` across one source's uncompressed bytes."""
    pinned = open_pinned_source(source)
    try:
        yield from iter_snapshot_lines([pinned])
    finally:
        pinned[1].close()


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


def discover_sources(root: Path) -> list[RawSource]:
    """Every raw source of every install, ordered by install then sequence."""
    sources: list[RawSource] = []
    if not root.is_dir():
        return sources
    for directory in sorted(root.iterdir()):
        if not directory.is_dir() or not INSTALL_ID_RE.fullmatch(directory.name):
            continue
        sources.extend(raw_sources(directory))
    return sources


def discover_install_directories(root: Path) -> list[tuple[str, Path]]:
    if not root.is_dir():
        return []
    return [
        (directory.name.lower(), directory)
        for directory in sorted(root.iterdir())
        if directory.is_dir() and INSTALL_ID_RE.fullmatch(directory.name)
    ]


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
    for install_id, directory in discover_install_directories(root):
        if processed >= max_lines:
            break
        # One pinned snapshot per install: a concurrent seal can never make a
        # checkpoint key read another generation's bytes.
        snapshot = open_raw_snapshot(directory)
        try:
            if not snapshot:
                continue
            metadata = _load_metadata(root, install_id)
            state = _load_state(root, install_id)
            if state is not None:
                store.update_state(install_id, state, metadata=metadata)
            for source, handle, snapshot_size in snapshot:
                if processed >= max_lines:
                    break
                relative = source.key
                mtime_ns = _handle_identity(handle)[2]
                offset = store.checkpoint_offset(relative)
                if offset > snapshot_size:
                    raise StoreError("backfill source shrank; operator reset is required")
                if offset == snapshot_size:
                    # Already imported; seeking a gzip segment would decompress it.
                    continue
                handle.seek(offset)
                carried = None
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
                        if carried is not None:
                            raw, consumed, oversized = carried
                            carried = None
                        else:
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
                            # Carry the line instead of seeking back: gzip
                            # sources cannot rewind without re-decompressing.
                            carried = (raw, consumed, oversized)
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
                        source_mtime_ns=mtime_ns,
                        complete=batch_end >= snapshot_size,
                        metadata=metadata,
                    )
                    offset = batch_end
                    processed += raw_count
                    inserted += result.inserted
                    duplicates += result.duplicates
                    malformed += malformed_count
        finally:
            close_snapshot(snapshot)
    reports = store.checkpoint_reports()
    finished = {
        report["source_path"]
        for report in reports
        if report["complete"] == 1
        and report["byte_offset"] == report["source_size"]
    }
    sources = discover_sources(root)
    complete = bool(sources) and all(
        source.key in finished or raw_source_size(source) == 0 for source in sources
    )
    return {
        "schema": "murmur-event-backfill/v1",
        "processed_lines": processed,
        "inserted_events": inserted,
        "duplicate_events": duplicates,
        "malformed_lines": malformed,
        "complete": complete,
    }


def _scan_reconciliation_sources(
    install_id: str,
    snapshot: Sequence[tuple[RawSource, object, int]],
    temporary_directory: str,
) -> dict:
    raw_lines = valid_objects = malformed_lines = duplicates = untimed_events = 0
    earliest = latest = None
    dedupe_path = os.path.join(temporary_directory, "hashes.sqlite3")
    dedupe = sqlite3.connect(dedupe_path)
    dedupe.execute("PRAGMA journal_mode = OFF")
    dedupe.execute("PRAGMA synchronous = OFF")
    dedupe.execute("CREATE TABLE hashes(value TEXT PRIMARY KEY) WITHOUT ROWID")
    source_sizes = {}
    for source, handle, snapshot_size in snapshot:
        source_sizes[source.key] = snapshot_size
        offset = 0
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
        "source_sizes": source_sizes,
        "hash_set_digest": digest.digest(),
    }


def reconcile(store: EventStore, root: Path, *, mark_ready: bool = False) -> dict:
    reports = []
    all_ready = True
    connection = store._connect(write=False)
    try:
        for install_id, install_directory in discover_install_directories(root):
            snapshot = open_raw_snapshot(install_directory)
            try:
                if not snapshot:
                    continue
                with tempfile.TemporaryDirectory(prefix="murmur-reconcile-") as directory:
                    source = _scan_reconciliation_sources(install_id, snapshot, directory)
            finally:
                close_snapshot(snapshot)
            sources = [item[0] for item in snapshot]
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
            inserted_events = 0
            checkpoints_complete = True
            for key, size in source["source_sizes"].items():
                checkpoint = connection.execute(
                    """
                    SELECT inserted_events, byte_offset, source_size, complete
                    FROM backfill_checkpoints WHERE source_path = ?
                    """,
                    (key,),
                ).fetchone()
                if checkpoint is None:
                    # An empty source (e.g. a live file whose only line the
                    # scrub removed) has nothing to backfill.
                    checkpoints_complete = checkpoints_complete and size == 0
                    continue
                inserted_events += int(checkpoint["inserted_events"])
                checkpoints_complete = checkpoints_complete and (
                    checkpoint["complete"] == 1
                    and checkpoint["byte_offset"] == size
                    and checkpoint["source_size"] == size
                )
            database_count = int(row["count"])
            expected_count = source["valid_objects"] - source["duplicates"]
            ready = (
                database_count == expected_count
                and int(row["untimed"] or 0) == source["untimed_events"]
                and row["earliest"] == source["earliest_time_us"]
                and row["latest"] == source["latest_time_us"]
                and checkpoints_complete
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
                    "raw_sources": len(sources),
                    "inserted_events": inserted_events,
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
