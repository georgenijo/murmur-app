from __future__ import annotations

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load_script(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / 'infra/native-capture-ci' / f'{name}.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


poller = load_script('poll_capture')
controller = load_script('run_capture')


class NativeCaptureCiTests(unittest.TestCase):
    def test_source_requires_sha_and_trusted_ref_shape(self):
        controller.validate_source('a' * 40, 'main')
        controller.validate_source('a' * 40, 'issue/448-first-pcm-capture-smoke')
        for sha, ref in [('main','main'), ('a'*40,'refs/pull/1/head'), ('a'*40,'x; touch injected'), ('A'*40,'main')]:
            with self.subTest(sha=sha,ref=ref), self.assertRaises(ValueError):
                controller.validate_source(sha,ref)

    def test_capture_paths(self):
        for path in ['app/src-tauri/sidecars/capture/src/production.rs',
                     'app/src-tauri/crates/capture-helper-protocol/src/lib.rs',
                     'app/src-tauri/src/audio.rs', 'app/src-tauri/src/audio_lifecycle.rs',
                     'app/src-tauri/Cargo.lock', 'scripts/smoke_test_capture_first_pcm.py']:
            self.assertTrue(poller.capture_path(path), path)
        for path in ['docs/FEATURES.md','app/src/App.tsx','app/src-tauri/src/history.rs']:
            self.assertFalse(poller.capture_path(path), path)

    def test_private_workflow_never_runs_public_pull_requests(self):
        workflow = (ROOT/'infra/native-capture-ci/.github/workflows/capture-smoke.yml').read_text()
        self.assertIn("github.repository == 'georgenijo/murmur-native-ci'", workflow)
        self.assertIn("github.actor == 'georgenijo'", workflow)
        self.assertIn("github.triggering_actor == 'georgenijo'", workflow)
        self.assertNotIn('pull_request', workflow)
        self.assertNotIn('secrets.', workflow)
        self.assertIn('contents: read', workflow)
        self.assertIn('persist-credentials: false', workflow)

    def git(self, *args):
        return subprocess.run(['git',*args],cwd=self.repo,text=True,capture_output=True,check=True).stdout.strip()

    def setUp(self):
        directory=tempfile.TemporaryDirectory(); self.addCleanup(directory.cleanup)
        self.root=Path(directory.name)
        self.repo=self.root/'repo'; self.repo.mkdir()
        self.git('init','--initial-branch=main')
        self.git('config','user.name','CI test')
        self.git('config','user.email','ci@example.invalid')
        self.git('config','commit.gpgsign','false')
        self.commit('README.md')
        self.state=self.root/'state'
        self.calls=[]
        real_run=poller.run
        def run(*args,cwd,input=None):
            if args[0]=='fake-gh':
                self.calls.append((args,input))
                if args[1:3]==('run','list'): return '[]'
                return ''
            return real_run(*args,cwd=cwd,input=input)
        self.addCleanup(patch.stopall)
        patch.object(poller,'SOURCE',str(self.repo)).start()
        patch.object(poller,'run',side_effect=run).start()

    def commit(self,path):
        target=self.repo/path; target.parent.mkdir(parents=True,exist_ok=True)
        target.write_text(target.read_text()+'changed\n' if target.exists() else 'new\n')
        self.git('add',path); self.git('commit','--quiet','-m','test change')
        return self.git('rev-parse','HEAD')

    def test_first_run_and_repeat_are_idempotent(self):
        first=poller.poll(self.state,'fake-gh')
        second=poller.poll(self.state,'fake-gh')
        self.assertTrue(first['dispatched'])
        self.assertFalse(second['dispatched'])
        dispatch=[c for c in self.calls if c[0][1:3]==('workflow','run')]
        self.assertEqual(len(dispatch),1)
        self.assertEqual(json.loads(dispatch[0][1]),{'source_sha':self.git('rev-parse','HEAD'),'source_ref':'main'})

    def test_non_capture_change_skips_then_capture_change_dispatches(self):
        poller.poll(self.state,'fake-gh')
        self.commit('docs/FEATURES.md')
        self.assertFalse(poller.poll(self.state,'fake-gh')['dispatched'])
        self.commit('app/src-tauri/src/audio.rs')
        self.assertTrue(poller.poll(self.state,'fake-gh')['dispatched'])

    def test_failed_dispatch_does_not_advance_state(self):
        poller.poll(self.state,'fake-gh')
        before=(self.state/'state.json').read_text()
        self.commit('app/src-tauri/src/audio.rs')
        original=poller.run
        def fail(*args,**kwargs):
            if args[0:3]==('fake-gh','workflow','run'):
                raise subprocess.CalledProcessError(1,args)
            return original(*args,**kwargs)
        with patch.object(poller,'run',side_effect=fail), self.assertRaises(subprocess.CalledProcessError):
            poller.poll(self.state,'fake-gh')
        self.assertEqual((self.state/'state.json').read_text(),before)

    def test_rename_out_of_capture_paths_dispatches(self):
        watched = 'app/src-tauri/src/audio.rs'
        destination = 'app/src-tauri/src/renamed_capture.rs'
        prior = self.commit(watched)
        poller.poll(self.state, 'fake-gh')
        self.git('mv', watched, destination)
        self.git('commit', '--quiet', '-m', 'rename capture source')
        current = self.git('rev-parse', 'HEAD')
        self.assertEqual(self.git('diff', '--name-only', '--find-renames', prior, current),
                         destination)
        result = poller.poll(self.state, 'fake-gh')
        self.assertTrue(result['capture_paths_changed'])
        self.assertTrue(result['dispatched'])

    def test_already_dispatched_sha_does_not_duplicate_after_state_loss(self):
        sha=self.git('rev-parse','HEAD')
        original=poller.run
        def existing(*args,**kwargs):
            if args[0:3]==('fake-gh','run','list'):
                return json.dumps([{'displayTitle':f'Capture smoke {sha}'}])
            return original(*args,**kwargs)
        with patch.object(poller,'run',side_effect=existing):
            self.assertFalse(poller.poll(self.state,'fake-gh')['dispatched'])
        self.assertEqual(json.loads((self.state/'state.json').read_text())['last_seen_sha'],sha)


if __name__=='__main__':
    unittest.main()
