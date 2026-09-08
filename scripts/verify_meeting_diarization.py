#!/usr/bin/env python3
"""Exercise the shipped signed worker with an explicit local multi-speaker fixture.

Leaves transcript/audio/model content out of the report. Run in an exclusive
performance slot. The fixture must be 16 kHz mono WAV, such as AMI ES2004a
Mix-Headset. The models must already have been installed through Murmur.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import resource
import signal
import statistics
import subprocess
import sys
import threading
import time
import wave

PREFIX = b'MRMR_DIARIZATION_V1 '
LIMIT = 4 * 1024 * 1024


def worker(executable, fixture, duration_ms, close_parent=False):
    started = time.monotonic()
    process = subprocess.Popen([str(executable), '--meeting-diarization-worker-v1'],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, start_new_session=True)
    output = bytearray()

    def drain():
        while block := process.stdout.read(65536):
            output.extend(block)
            if len(output) > LIMIT:
                os.killpg(process.pid, signal.SIGKILL)
                return

    reader = threading.Thread(target=drain, daemon=True)
    reader.start()
    process.stdin.write((json.dumps({'audioPath': str(fixture), 'durationMs': duration_ms}) + '\n').encode())
    process.stdin.flush()
    if close_parent:
        process.stdin.close()
    try:
        process.wait(timeout=120)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)
        raise AssertionError('worker exceeded deadline')
    finally:
        if not process.stdin.closed:
            process.stdin.close()
    reader.join(timeout=5)
    assert not reader.is_alive(), 'worker left stdout owner alive'
    try:
        os.killpg(process.pid, 0)
    except ProcessLookupError:
        pass
    else:
        raise AssertionError('worker process group still exists')
    elapsed = time.monotonic() - started
    if close_parent:
        assert process.returncode != 0, 'parent-death worker unexpectedly completed'
        assert elapsed < 5, 'parent death did not promptly terminate worker'
        return {'parentDeathSeconds': round(elapsed, 3)}, None
    assert process.returncode == 0, f'worker failed with content-free exit code {process.returncode}'
    payloads = [line[len(PREFIX):] for line in bytes(output).splitlines() if line.startswith(PREFIX)]
    assert len(payloads) == 1, 'worker did not publish exactly one receipt'
    turns = json.loads(payloads[0])
    assert 0 < len(turns) <= 30000
    identities = {}
    canonical = []
    for turn in turns:
        assert set(turn) == {'speaker', 'startMs', 'endMs', 'quality'}
        assert 0 <= turn['speaker'] < 32
        assert 0 <= turn['startMs'] < turn['endMs'] <= duration_ms + 100
        assert 0 <= turn['quality'] <= 1
        identity = identities.setdefault(turn['speaker'], len(identities))
        canonical.append([identity, turn['startMs'], turn['endMs']])
    digest = hashlib.sha256(json.dumps(canonical, separators=(',', ':')).encode()).hexdigest()
    return {'elapsedSeconds': round(elapsed, 3), 'speakerCount': len(identities),
            'turnCount': len(turns), 'boundaryDigest': digest}, canonical


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--executable', type=Path, required=True)
    parser.add_argument('--fixture', type=Path, required=True)
    parser.add_argument('--expected-speakers', type=int, default=4)
    parser.add_argument('--asr-fixture', type=Path, help='Optional <=30s 16kHz mono PCM fixture for the debug-only production-priority check')
    parser.add_argument('--report', type=Path, required=True)
    parser.add_argument('--seed-gui', action='store_true', help='Debug-only: seed two real-label fixture meetings in the empty isolated 540 test store')
    args = parser.parse_args()
    executable = args.executable.resolve(strict=True)
    fixture = args.fixture.resolve(strict=True)
    if sys.platform != 'darwin':
        parser.error('production diarization verification requires macOS')
    subprocess.run(['/usr/bin/codesign', '--verify', '--strict', str(executable)], check=True)
    with wave.open(str(fixture)) as audio:
        assert audio.getframerate() == 16000 and audio.getnchannels() == 1
        duration_ms = round(audio.getnframes() * 1000 / audio.getframerate())
    assert 0 < duration_ms <= 7200000
    first, boundaries = worker(executable, fixture, duration_ms)
    second, repeated = worker(executable, fixture, duration_ms)
    assert first['speakerCount'] == args.expected_speakers
    assert second['speakerCount'] == args.expected_speakers
    assert boundaries == repeated, 'speaker labels or boundaries changed between repeated fixture passes'
    death, _ = worker(executable, fixture, duration_ms, close_parent=True)
    report = {'schema': 'murmur.diarization-worker-check.v1', 'passed': True,
              'executableSha256': hashlib.sha256(executable.read_bytes()).hexdigest(),
              'fixtureSha256': hashlib.sha256(fixture.read_bytes()).hexdigest(),
              'durationMs': duration_ms, 'passes': [first, second], **death,
              'childPeakRssBytes': resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss}
    if args.asr_fixture:
        asr_fixture = args.asr_fixture.resolve(strict=True)
        assert asr_fixture.stat().st_size <= 1_000_000
        request = json.dumps({'audioPath': str(fixture), 'durationMs': duration_ms,
                              'asrAudioPath': str(asr_fixture)}) + '\n'
        check = subprocess.run([str(executable), '--meeting-diarization-preemption-check-v1'],
                               input=request, text=True, capture_output=True, timeout=420,
                               start_new_session=True)
        assert check.returncode == 0, f'priority check failed with content-free code {check.returncode}'
        prefix = 'MRMR_DIARIZATION_PRIORITY_V1 '
        receipts = [json.loads(line[len(prefix):]) for line in check.stdout.splitlines() if line.startswith(prefix)]
        assert len(receipts) == 1
        priority = receipts[0]
        trials = priority['trials']
        assert len(trials) == 3
        assert all(t['terminationConfirmed'] and t['textUnchanged'] and t['cancellationMs'] <= 150 for t in trials)
        baseline = statistics.median(t['baselineDecodeMs'] for t in trials)
        candidate = statistics.median(t['priorityDecodeMs'] for t in trials)
        priority['medianDecodeDeltaPercent'] = (candidate - baseline) / baseline * 100
        priority['asrFixtureSha256'] = hashlib.sha256(asr_fixture.read_bytes()).hexdigest()
        report['priority'] = priority
    if args.seed_gui:
        store_root = Path.home() / 'Library/Application Support/com.localdictation.diarization540/meetings'
        if store_root.exists() and any(store_root.iterdir()):
            raise AssertionError('The isolated 540 fixture store is not empty; refusing to overwrite it')
        request = json.dumps({'audioPath': str(fixture), 'storeRoot': str(store_root)}) + '\n'
        seeded = subprocess.run([str(executable), '--meeting-diarization-seed-fixture-v1'],
                                input=request, text=True, capture_output=True, timeout=150,
                                start_new_session=True)
        assert seeded.returncode == 0, f'GUI fixture refused or failed with content-free code {seeded.returncode}'
        prefix = 'MRMR_DIARIZATION_GUI_FIXTURE_V1 '
        receipts = [json.loads(line[len(prefix):]) for line in seeded.stdout.splitlines() if line.startswith(prefix)]
        assert len(receipts) == 1 and receipts[0]['labelsFromNativeWorker']
        report['guiFixture'] = receipts[0]
    args.report.write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
