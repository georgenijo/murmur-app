#!/usr/bin/env python3
"""Native AX-only fixture review. No HID, activation, VoiceOver, TCC or user data.

Traverses, presses and closes only the exact task-owned child PID. Text focus
and entry use the QA app's internal hook, never synthetic OS keyboard input.
"""
from pathlib import Path
import json
import argparse
import os
import platform
import subprocess
import time
from check_gpui_goldens import check_inventory
from check_gpui_onboarding_home_native import OBSERVER

ROOT = Path(__file__).resolve().parents[1]


def main():
    if platform.system() != 'Darwin':
        raise SystemExit('native meetings verification requires macOS')
    ui = ROOT / 'app/gpui'
    target = Path(json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--no-deps', '--format-version=1'], cwd=ui))['target_directory'])
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, help='Exact gallery QA binary, including one inside a review app bundle')
    parser.add_argument('--evidence', type=Path, help='Separate output directory for packaged/unbundled receipts')
    args = parser.parse_args()
    evidence = args.evidence or target / 'meetings-native'
    evidence.mkdir(parents=True, exist_ok=True)
    source = evidence / 'observer.m'
    source.write_text(OBSERVER)
    observer = evidence / 'observer'
    subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit',
                    '-framework', 'ApplicationServices', str(source), '-o', str(observer)], check=True)
    env = {k: v for k, v in os.environ.items() if not k.startswith('MURMUR_')}
    binary = str((args.binary or target / 'debug/meetings-qa').resolve())
    observations = []
    # Synthetic Calendar candidates only; selection remains explicit, including
    # the one-event case. Naming, speaker edits and export never touch real data.
    steps = [('find', 'Notetaker'), ('find', 'Search meetings'),
             ('press', 'Play'), ('find', 'Pause'), ('press', 'Pause'),
             ('find', 'Resume'), ('press', 'Rename'), ('find', 'Meeting title'),
             ('press', 'Cancel naming'), ('press', 'Name from calendar…'),
             ('find', 'Choose an overlapping event'),
             ('press', 'Synthetic calendar event · 1700000000000 – 1700000065000 · Attendees: Fixture guest'),
             ('press', 'Apply event'), ('find', 'Meeting named from calendar.'),
             ('press', 'Start meeting'), ('find', 'Recording meeting'),
             ('press', 'Stop meeting'), ('find', 'Finishing pending transcript chunks…')]
    try:
        with (evidence / 'meetings-host.log').open('w') as log:
            host = subprocess.Popen([binary], cwd=ui, env=env, stdout=log, stderr=log)
            try:
                for action, label in steps:
                    deadline = time.monotonic() + 15
                    while True:
                        if host.poll() is not None:
                            raise RuntimeError('synthetic meetings test process exited')
                        result = subprocess.run([str(observer), str(host.pid), action, label],
                                                capture_output=True, text=True, timeout=8)
                        if result.returncode == 0:
                            observations.append(dict(pid=host.pid, action=action, label=label))
                            break
                        if result.returncode == 77:
                            raise RuntimeError(result.stderr.strip())
                        if time.monotonic() >= deadline:
                            raise RuntimeError(f'AX {action} {label}: {result.returncode}')
                        time.sleep(0.1)
                    if action == 'find':
                        window = subprocess.check_output([str(observer), str(host.pid), 'window', '-'], text=True).strip()
                        subprocess.run(['screencapture', '-x', '-o', '-l', window,
                                        str(evidence / f'flow-{len(observations):02}.png')], check=True)
                    time.sleep(0.2)
                subprocess.run([str(observer), str(host.pid), 'close', '-'], timeout=8)
                host.wait(timeout=15)
                if host.returncode:
                    raise RuntimeError(f'native test exited {host.returncode}')
                check_inventory((evidence / 'meetings-host.log').read_text(), 'MURMUR_MEETINGS_INVENTORY')
            finally:
                if host.poll() is None:
                    host.terminate()
                    host.wait(timeout=15)
        for name, flag in [('text-focus', 'MURMUR_NATIVE_TEXT_CHECK'), ('audio', 'MURMUR_AUDIO_SELF_TEST')]:
            run = subprocess.run([binary], cwd=ui, env={**env, flag: '1'}, text=True,
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
            (evidence / f'{name}.log').write_text(run.stdout)
            check_inventory(run.stdout, 'MURMUR_MEETINGS_INVENTORY')
            run.check_returncode()
            observations.append(dict(action=name, result='pass'))
    finally:
        (evidence / 'receipt.json').write_text(json.dumps({'observations': observations,
            'method': 'native AXPress/read; exact child PID; internal text hook; synthetic PCM'}, indent=2) + '\n')
    print(f'PASS native meetings fixture flow: {len(observations)} observations')


if __name__ == '__main__':
    main()
