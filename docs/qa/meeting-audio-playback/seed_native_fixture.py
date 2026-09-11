#!/usr/bin/env python3
"""Add synthetic audio only to the isolated Meetings smoke-test app's store.

This checks playback independently of macOS capture permission. It does not
produce evidence that recording succeeded. Stop Murmur Meetings Test first.
"""

import array
import json
import math
import sqlite3
import subprocess
import sys
import time
import wave
from pathlib import Path


BUNDLE_ID = "com.localdictation.meetings716"
SESSION_ID = "synthetic-audio-playback-724"
TITLE = "Synthetic playback 724 (not a recording)"
ROOT = Path.home() / "Library/Application Support" / BUNDLE_ID / "meetings"
CLIPS = [
    ("me", 0, 0, 4000, 220),
    ("them", 0, 7000, 12000, 660),
    ("me", 1, 9000, 13000, 330),
    ("them", 1, 17000, 20000, 880),
]


def main():
    if sys.platform != "darwin":
        raise SystemExit("This fixture is for the isolated macOS smoke-test app.")
    processes = subprocess.check_output(["ps", "-ax", "-o", "command="], text=True)
    if "Murmur Meetings Test.app/Contents/MacOS/ui" in processes:
        raise SystemExit("Stop Murmur Meetings Test before seeding its store.")
    database = ROOT / "meetings.sqlite3"
    if not database.is_file() or database.is_symlink() or ROOT.resolve() != ROOT:
        raise SystemExit("Expected a real, initialized isolated smoke-test store.")
    connection = sqlite3.connect(database)
    connection.execute("PRAGMA foreign_keys=ON")
    if connection.execute("PRAGMA user_version").fetchone()[0] != 5:
        raise SystemExit("Review fixture compatibility before using another schema.")
    if connection.execute("SELECT 1 FROM meeting_sessions WHERE id=?", [SESSION_ID]).fetchone():
        raise SystemExit("Fixture already exists; delete it in the app before reseeding.")
    directory = ROOT / "audio" / SESSION_ID
    if directory.exists():
        raise SystemExit("Fixture audio directory already exists; inspect it first.")
    directory.mkdir(parents=True)
    paths = []
    committed = False
    try:
        for channel, sequence, start_ms, end_ms, frequency in CLIPS:
            path = directory / f"{channel}-{sequence:08}.wav"
            paths.append(path)
            samples = array.array("h", (
                round(4096 * math.sin(2 * math.pi * frequency * index / 16000))
                for index in range((end_ms - start_ms) * 16)
            ))
            if sys.byteorder != "little":
                samples.byteswap()
            with wave.open(str(path), "wb") as output:
                output.setnchannels(1)
                output.setsampwidth(2)
                output.setframerate(16000)
                output.writeframes(samples.tobytes())
        started = int(time.time() * 1000) - 20000
        with connection:
            connection.execute(
                "INSERT INTO meeting_sessions(id,started_at_ms,ended_at_ms,status,model_name,language,smart_punctuation,retain_audio,title,title_source) "
                "VALUES (?,?,?,'complete','synthetic-fixture','en',0,1,?,'manual')",
                [SESSION_ID, started, started + 20000, TITLE],
            )
            connection.execute("INSERT INTO meeting_sessions_fts(session_id,title) VALUES (?,?)", [SESSION_ID, TITLE])
            for (channel, sequence, start_ms, end_ms, frequency), path in zip(CLIPS, paths):
                text = f"Synthetic {channel.title()} channel: {frequency} Hz tone from {start_ms / 1000:g} to {end_ms / 1000:g} seconds. Not captured audio."
                cursor = connection.execute(
                    "INSERT INTO meeting_segments(session_id,speaker,sequence,start_ms,end_ms,status,text,audio_relative_path) "
                    "VALUES (?,?,?,?,?,'final',?,?)",
                    [SESSION_ID, channel, sequence, start_ms, end_ms, text, str(path.relative_to(ROOT))],
                )
                connection.execute("INSERT INTO meeting_segments_fts(segment_id,session_id,text) VALUES (?,?,?)", [cursor.lastrowid, SESSION_ID, text])
        committed = True
        print(json.dumps({"sessionId": SESSION_ID, "title": TITLE, "synthetic": True, "durationMs": 20000, "files": [str(path.relative_to(ROOT)) for path in paths]}, indent=2))
    except BaseException:
        if not committed:
            for path in paths:
                path.unlink(missing_ok=True)
            directory.rmdir()
        raise
    finally:
        connection.close()


if __name__ == "__main__":
    main()
