#!/usr/bin/env python3
"""AX-only remaining-section checks. Exact child PID, no HID/TCC/VoiceOver."""
from pathlib import Path
import argparse
import json
import os
import platform
import subprocess
import time
from check_gpui_goldens import check_inventory
from check_gpui_onboarding_home_native import OBSERVER
ROOT = Path(__file__).resolve().parents[1]

def main():
    if platform.system() != 'Darwin':
        raise SystemExit('native verification requires macOS')
    ui = ROOT / 'app/gpui'
    target = Path(json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--no-deps', '--format-version=1'], cwd=ui))['target_directory'])
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path)
    parser.add_argument('--evidence', type=Path)
    args = parser.parse_args()
    evidence = args.evidence or target / 'remaining-native'
    evidence.mkdir(parents=True, exist_ok=True)
    (evidence / 'observer.m').write_text(OBSERVER)
    observer = evidence / 'observer'
    subprocess.run(['clang', '-fobjc-arc', '-Werror', '-framework', 'AppKit',
                    '-framework', 'ApplicationServices', str(evidence / 'observer.m'),
                    '-o', str(observer)], check=True)
    binary = str((args.binary or target / 'debug/remaining-qa').resolve())
    env = {k: v for k, v in os.environ.items() if not k.startswith('MURMUR_')}
    steps = [('find', 'Search commands'), ('press', 'Review Insights'),
             ('find', 'Usage totals'), ('press', 'Review Models'),
             ('find', 'Model storage'), ('press', 'Review Knowledge'),
             ('press', 'Create knowledge…'), ('find', 'Knowledge type'),
             ('press', 'Cancel'), ('press', 'Delete all…'),
             ('find', 'Type DELETE to confirm'), ('press', 'Cancel'),
             ('press', 'Review Saved spellings'), ('press', 'Add spelling'),
             ('find', 'Murmur types'), ('press', 'Cancel'),
             ('press', 'Review Voice commands'), ('press', 'Duplicate voice command'),
             ('find', 'Spoken phrase'), ('press', 'Cancel'),
             ('press', 'Review App overrides'), ('press', 'Add app manually…'),
             ('find', 'App bundle ID'), ('press', 'Cancel'),
             ('press', 'Review Project vocabulary'), ('press', 'Scan folder'),
             ('find', 'Copy terms'),
             ('press', 'Review Modes'), ('press', 'Create mode'),
             ('press', 'Edit mode…'), ('find', 'Mode model'),
             ('press', 'Cancel edit'), ('press', 'Review Themes'),
             ('press', 'Import theme…'), ('press', 'Browse community…'),
             ('find', 'Search community themes'), ('press', 'Cancel'),
             ('press', 'Import theme…'), ('press', 'Choose local file…'),
             ('find', 'Theme import preview'), ('press', 'Cancel')]
    observations, failures = [], []
    recording = None
    try:
        with (evidence / 'native-host.log').open('w') as log:
            host = subprocess.Popen([binary], cwd=ui, env=env, stdout=log, stderr=log)
            try:
                for action, label in steps:
                    deadline = time.monotonic() + 12
                    while True:
                        if host.poll() is not None:
                            raise RuntimeError('synthetic review process exited')
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
                    if recording is None:
                        window = subprocess.check_output([str(observer), str(host.pid), 'window', '-'], text=True).strip()
                        recording = subprocess.Popen(['screencapture', '-x', '-v', '-l', window,
                                                      '-V', '20', str(evidence / 'native-flow.mov')],
                                                     stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
                    if action == 'find':
                        window = subprocess.check_output([str(observer), str(host.pid), 'window', '-'], text=True).strip()
                        subprocess.run(['screencapture', '-x', '-o', '-l', window,
                                        str(evidence / f'flow-{len(observations):02}.png')], check=True)
                    time.sleep(0.3)
                if recording is not None:
                    recording.wait(timeout=25)
                subprocess.run([str(observer), str(host.pid), 'close', '-'], timeout=8)
                host.wait(timeout=15)
                if host.returncode:
                    failures.append(f'packaged/native launch policy exit {host.returncode}')
            finally:
                if host.poll() is None:
                    host.terminate()
                    host.wait(timeout=15)
                if recording is not None and recording.poll() is None:
                    recording.terminate()
                    recording.wait(timeout=10)
        try:
            check_inventory((evidence / 'native-host.log').read_text(), 'MURMUR_REMAINING_INVENTORY')
        except (AssertionError, KeyError) as error:
            failures.append(str(error))
        run = subprocess.run([binary], cwd=ui, env={**env, 'MURMUR_NATIVE_TEXT_CHECK': '1'},
                             text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=30)
        (evidence / 'text-focus.log').write_text(run.stdout)
        lines = [line.split('=', 1)[1] for line in run.stdout.splitlines()
                 if line.startswith('MURMUR_REMAINING_TEXT=')]
        if len(lines) != 1:
            failures.append('missing native palette input receipt')
        else:
            text = json.loads(lines[0])
            if text['before'] != text['after']:
                failures.append('focus/typing loaded Writing Tools')
            observations.append(dict(action='palette-focus-input', **text))
        try:
            check_inventory(run.stdout, 'MURMUR_REMAINING_INVENTORY')
        except (AssertionError, KeyError) as error:
            failures.append(str(error))
        if run.returncode:
            failures.append(f'text probe policy exit {run.returncode}')
    finally:
        (evidence / 'receipt.json').write_text(json.dumps({'method': 'AXPress/read; exact child PID; internal native text handler',
            'observations': observations, 'failures': failures}, indent=2) + '\n')
    if failures:
        raise SystemExit('\n'.join(failures))
    print(f'PASS native remaining sections: {len(observations)} observations')
if __name__ == '__main__':
    main()
