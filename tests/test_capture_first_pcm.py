from __future__ import annotations

import os
from pathlib import Path
import stat
import tempfile
import time
import unittest
from scripts.smoke_test_capture_first_pcm import SmokeError, run_backend, select_builtin_input


WORKER = r'''#!/usr/bin/env python3
import json, os, signal, struct, sys, time
capture_id, nonce = int(sys.argv[2]), bytes.fromhex(sys.argv[3])
assert sys.argv[1] == '--production-v10'
scenario = SCENARIO

def read():
    h = sys.stdin.buffer.read(36)
    magic, version, kind, channel, size, cid, n = struct.unpack('<4sHBBIQ16s', h)
    assert (magic, version, kind, channel, cid, n) == (b'MRMR', 10, 0, 0, capture_id, nonce)
    return json.loads(sys.stdin.buffer.read(size))

def emit(payload, kind=0, channel=0):
    if kind == 0:
        payload = json.dumps(payload).encode()
    sys.stdout.buffer.write(struct.pack('<4sHBBIQ16s', b'MRMR', 10, kind, channel, len(payload), capture_id, nonce) + payload)
    sys.stdout.buffer.flush()

assert read() == {'type': 'hello'}
if scenario == 'partial_header':
    sys.stdout.buffer.write(b'MRM'); sys.stdout.buffer.flush(); time.sleep(30)
if scenario == 'oversize':
    sys.stdout.buffer.write(struct.pack('<4sHBBIQ16s', b'MRMR', 10, 0, 0, 1000000, capture_id, nonce)); sys.stdout.buffer.flush(); time.sleep(30)
emit({'type':'helloAck'})
command = read()
if command['type'] == 'enumerate':
    devices = [{'id':'private-built-in-uid','name':'private name','kind':'builtIn','connected':True,'hasInput':True}]
    if scenario in ['ambiguous', 'builtin_default']:
        devices.append(dict(devices[0], id='second-private-builtin-uid'))
    if scenario == 'continuity': devices[0]['kind'] = 'continuity'
    emit({'type':'devices','devices':devices,'defaultInputId':'private-built-in-uid' if scenario == 'builtin_default' else 'unrelated-default','lidState':'closed' if scenario == 'closed' else 'open'})
    sys.exit(0)
backend = command['backend']
assert command['deviceId'] == 'private-built-in-uid'
if scenario == 'hang':
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    if hasattr(os, 'fork'):
        child = os.fork()
        if child:
            open(PIDFILE, 'w').write(str(child))
    time.sleep(30)
emit({'type':'phase','backend':backend,'phase':'streamOpen'})
if scenario in ['early_awaiting', 'early_active']:
    emit({'type':'phase','backend':backend,'phase':'awaitingFirstCallback'})
if scenario == 'early_active':
    emit({'type':'phase','backend':backend,'phase':'active'})
steps = ['deviceResolution','defaultConfig','streamBuild','streamStart','awaitingFirstCallback'] if backend == 'cpal' else ['deviceResolution','audioUnitNew','enableInputIo','disableOutputIo','setCurrentDevice','formatConfiguration','callbackInstallation','streamStart','awaitingFirstCallback']
if scenario == 'order': steps[1:3] = reversed(steps[1:3])
for step in steps:
    for transition in ['entered','completed']:
        if scenario == 'unbalanced' and step == 'streamStart' and transition == 'completed': continue
        emit({'type':'setupStep','backend':backend,'step':step,'transition':transition})
        if step == 'deviceResolution' and transition == 'entered' and scenario != 'missing_resolution':
            emit({'type':'inputResolution','backend':backend,'inputEnumerationOk':True,'requestedPresent':True})
        if step == 'awaitingFirstCallback' and transition == 'entered' and scenario not in ['early_awaiting', 'early_active', 'late_awaiting']:
            emit({'type':'phase','backend':backend,'phase':'awaitingFirstCallback'})
if scenario == 'late_awaiting':
    emit({'type':'phase','backend':backend,'phase':'awaitingFirstCallback'})
if scenario != 'early_active':
    emit({'type':'phase','backend':'cpal' if scenario == 'fallback' else backend,'phase':'active'})
if scenario == 'late_pcm': time.sleep(.3)
if scenario == 'failure': emit({'type':'failure','backend':backend,'code':'private-device-secret'}); sys.exit(0)
for seq in range(3):
    emit(struct.pack('<QIIQQf', seq + (1 if scenario == 'sequence' else 0), 0 if scenario == 'sample_rate' else 16000, 1, seq, seq, .1), 1, 2 if scenario == 'system' else 1)
assert read() == {'type':'stop'}
if scenario == 'no_stop': time.sleep(30)
emit({'type':'stopped','retainedSamples':2 if scenario == 'retained' else 3})
if scenario == 'no_exit': time.sleep(30)
if scenario == 'bad_exit': sys.exit(7)
'''


class FirstPcmSmokeTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.worker = Path(self.directory.name) / 'worker'
        self.pidfile = Path(self.directory.name) / 'child.pid'

    def fixture(self, scenario):
        self.worker.write_text(WORKER.replace('SCENARIO', repr(scenario)).replace('PIDFILE', repr(str(self.pidfile))))
        self.worker.chmod(self.worker.stat().st_mode | stat.S_IXUSR)
        return self.worker

    def run_backend(self, scenario='ok', backend='auhal', **kwargs):
        return run_backend(self.fixture(scenario), backend, 'private-built-in-uid',
                           timeout=1.5, first_pcm_limit=.15, stop_limit=.15, **kwargs)

    def test_both_backends_require_real_framing_and_balanced_steps(self):
        for backend, count in [('auhal', 9), ('cpal', 5)]:
            with self.subTest(backend=backend):
                report = self.run_backend(backend=backend)
                self.assertEqual(report['pcm_frames'], 3)
                self.assertEqual(report['samples'], 3)
                self.assertEqual(report['setup_steps'], count)
                self.assertNotIn('private', str(report))

    def test_inventory_pins_builtin_even_when_default_differs(self):
        self.assertEqual(select_builtin_input(self.fixture('ok')), 'private-built-in-uid')

    def test_inventory_pins_builtin_default_when_multiple_exist(self):
        self.assertEqual(select_builtin_input(self.fixture('builtin_default')), 'private-built-in-uid')

    def test_unstable_inventory_fails_closed(self):
        for scenario in ['ambiguous', 'continuity', 'closed']:
            with self.subTest(scenario=scenario), self.assertRaises(SmokeError):
                select_builtin_input(self.fixture(scenario))

    def test_post_launch_initialization_failure_cleans_owned_process(self):
        import subprocess
        from unittest.mock import patch
        from scripts import smoke_test_capture_first_pcm as smoke
        real_popen = subprocess.Popen
        children = []
        def capture_child(*args, **kwargs):
            child = real_popen(*args, **kwargs)
            children.append(child)
            return child
        with patch.object(smoke.subprocess, 'Popen', side_effect=capture_child), \
                patch.object(smoke.os, 'set_blocking', side_effect=OSError('injected')):
            with self.assertRaises(OSError):
                self.run_backend()
        self.assertIsNotNone(children[0].returncode)

    def test_setup_and_backend_errors_fail(self):
        for scenario in ['order', 'unbalanced', 'fallback', 'missing_resolution']:
            with self.subTest(scenario=scenario), self.assertRaises(SmokeError):
                self.run_backend(scenario)

    def test_pcm_and_stop_mismatch_fail(self):
        for scenario in ['sequence', 'system', 'retained', 'sample_rate', 'bad_exit']:
            with self.subTest(scenario=scenario), self.assertRaises(SmokeError):
                self.run_backend(scenario)

    def test_phases_must_match_native_setup_position(self):
        for backend in ['auhal', 'cpal']:
            for scenario in ['early_awaiting', 'early_active', 'late_awaiting']:
                with self.subTest(backend=backend, scenario=scenario), self.assertRaises(SmokeError):
                    self.run_backend(scenario, backend=backend)

    def test_partial_and_oversized_headers_are_bounded(self):
        for scenario in ['partial_header', 'oversize']:
            started = time.monotonic()
            with self.subTest(scenario=scenario), self.assertRaises(SmokeError):
                self.run_backend(scenario)
            self.assertLess(time.monotonic() - started, 3)

    def test_startup_stop_and_exit_deadlines(self):
        for scenario in ['late_pcm', 'no_stop', 'no_exit']:
            started = time.monotonic()
            with self.subTest(scenario=scenario), self.assertRaises(SmokeError):
                self.run_backend(scenario)
            self.assertLess(time.monotonic() - started, 2)

    def test_process_observation_timeouts_use_fixed_errors(self):
        import subprocess
        from types import SimpleNamespace
        from unittest.mock import patch
        from scripts import smoke_test_capture_first_pcm as smoke
        timeout = subprocess.TimeoutExpired(['/bin/ps'], .1)
        with patch.object(smoke.subprocess, 'run', side_effect=timeout):
            with self.assertRaisesRegex(
                    SmokeError, '^worker did not exit after stop acknowledgment$'):
                smoke.wait_for_exit(SimpleNamespace(pid=123), time.monotonic() + .1)
            with self.assertRaisesRegex(
                    SmokeError, '^owned worker cleanup exceeded its deadline$'):
                smoke.live_group_members(123, .1)

    def test_worker_error_strings_do_not_escape(self):
        with self.assertRaises(SmokeError) as caught:
            self.run_backend('failure')
        self.assertNotIn('private-device-secret', str(caught.exception))

    @unittest.skipUnless(hasattr(os, 'fork'), 'requires POSIX process groups')
    def test_hung_worker_and_pipe_holding_child_are_killed(self):
        started = time.monotonic()
        with self.assertRaises(SmokeError):
            self.run_backend('hang')
        self.assertLess(time.monotonic() - started, 2)
        pid = int(self.pidfile.read_text())
        # A killed orphan may remain a zombie briefly; it must not be executing.
        import subprocess
        for _ in range(20):
            result = subprocess.run(['/bin/ps', '-o', 'stat=', '-p', str(pid)],
                                    capture_output=True, text=True)
            if not result.stdout.strip() or result.stdout.strip().startswith('Z'):
                break
            time.sleep(.025)
        else:
            self.fail('owned pipe-holding descendant survived cleanup')


if __name__ == '__main__':
    unittest.main()
