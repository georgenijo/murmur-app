"""Validate the production source boundary used by the private AX test runner."""
from pathlib import Path
import hashlib
import os
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from scripts import test_gpui


class TestSourceTests(unittest.TestCase):
    def test_expanded_adapter_patch_reproduces_the_vendored_files(self):
        root = test_gpui.UI
        # Pre-expansion adapter on PR #1027, head 0ad8c3af. node.rs already
        # includes the separately recorded Phase 0 selected-text bridge.
        original = {
            'src/node.rs': 'c7601b1541a5ff3f3102781278dc895027bb4646bec1c8f68746afee362cffa8',
            'src/event.rs': 'dca8b9ebcb6ffa268ad69caec92d1510136cddc8c1fb0615a3da29ee181a33a3',
        }
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            shutil.copytree(root / 'vendor/accesskit_macos', target, dirs_exist_ok=True)
            env = {**os.environ, 'GIT_CEILING_DIRECTORIES': str(target.parent)}
            patch_file = root / 'patches/accesskit-expanded.patch'
            # Layer 10 adds the separately recorded press probe to node.rs.
            # Remove the table and press layers before comparing layer 9's pin.
            heading_patch = root / 'patches/accesskit-heading-role.patch'
            subprocess.run(['git', 'apply', '--reverse', str(heading_patch)],
                           cwd=target, env=env, check=True)
            press_patch = root / 'patches/accesskit-press-probe.patch'
            tables_patch = root / 'patches/accesskit-tables.patch'
            subprocess.run(['git', 'apply', '--reverse', str(tables_patch)],
                           cwd=target, env=env, check=True)
            self.assertEqual(hashlib.sha256((target / 'src/node.rs').read_bytes()).hexdigest(),
                             '45ced32335b40ec92b928afb44ef44a8cd8beece2352a9db232b279c01b0bb95')
            subprocess.run(['git', 'apply', '--reverse', str(press_patch)],
                           cwd=target, env=env, check=True)
            subprocess.run(['git', 'apply', '--reverse', str(patch_file)],
                           cwd=target, env=env, check=True)
            for name, digest in original.items():
                self.assertEqual(hashlib.sha256((target / name).read_bytes()).hexdigest(), digest)
            subprocess.run(['git', 'apply', str(patch_file)], cwd=target, env=env, check=True)
            subprocess.run(['git', 'apply', str(press_patch)], cwd=target, env=env, check=True)
            subprocess.run(['git', 'apply', str(tables_patch)], cwd=target, env=env, check=True)
            subprocess.run(['git', 'apply', str(heading_patch)], cwd=target, env=env, check=True)
            for path in (root / 'vendor/accesskit_macos').rglob('*'):
                if path.is_file():
                    self.assertEqual((target / path.relative_to(root / 'vendor/accesskit_macos')).read_bytes(),
                                     path.read_bytes())

    def test_hover_source_patch_round_trips_the_frozen_gpui_file(self):
        root = test_gpui.UI
        patch_file = root / 'patches/gpui-hover-source.patch'
        original = root / 'vendor/gpui/src/elements/div.rs'
        input_patch = root / 'patches/gpui-input-active-descendant.patch'
        with tempfile.TemporaryDirectory() as directory:
            target = Path(directory)
            (target / 'src/elements').mkdir(parents=True)
            (target / 'src/elements/div.rs').write_bytes(original.read_bytes())
            (target / 'src/window').mkdir()
            a11y = root / 'vendor/gpui/src/window/a11y.rs'
            (target / 'src/window/a11y.rs').write_bytes(a11y.read_bytes())
            env = {**os.environ, 'GIT_CEILING_DIRECTORIES': str(target.parent)}
            subprocess.run(['git', 'apply', '--reverse', str(input_patch)],
                           cwd=target, env=env, check=True)
            subprocess.run(['git', 'apply', '--reverse', str(patch_file)],
                           cwd=target, env=env, check=True)
            self.assertEqual(hashlib.sha256((target / 'src/elements/div.rs').read_bytes()).hexdigest(),
                             '73b930227fe8f3c1fb703c2e7529d16a3f372cf52ec2afe95397ed0ff63f65d7')
            subprocess.run(['git', 'apply', str(patch_file)], cwd=target, env=env, check=True)
            subprocess.run(['git', 'apply', str(input_patch)], cwd=target, env=env, check=True)
            self.assertEqual((target / 'src/elements/div.rs').read_bytes(), original.read_bytes())
            self.assertEqual((target / 'src/window/a11y.rs').read_bytes(), a11y.read_bytes())

    def source(self, root, relative='vendor/gpui', revision=test_gpui.REV):
        source = root / relative
        source.mkdir(parents=True)
        manifest = source / 'Cargo.toml'
        manifest.write_text('[package.metadata.murmur]\nupstream-revision = "' + revision + '"\n')
        return source, {'manifest_path': str(manifest)}

    def test_neutral_overlay_geometry_fixture_matches_retained_frontend_fixture(self):
        import json
        root = Path(__file__).resolve().parents[1]
        native = json.loads((root / 'tests/fixtures/overlay-geometry.json').read_text())
        frontend = json.loads((root / 'app/src/components/overlay/overlay-geometry.fixture.json').read_text())
        self.assertEqual(native, frontend)

    def test_accepts_the_patched_production_source_at_the_frozen_revision(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source, package = self.source(root)
            with patch.object(test_gpui, 'UI', root):
                self.assertEqual(test_gpui.validated_source(package), source)

    def test_rejects_another_revision(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _, package = self.source(root, revision='different')
            with patch.object(test_gpui, 'UI', root), self.assertRaises(RuntimeError):
                test_gpui.validated_source(package)

    def test_rejects_a_test_only_or_shared_cargo_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            _, package = self.source(root, relative='cargo-checkout/gpui')
            with patch.object(test_gpui, 'UI', root), self.assertRaises(RuntimeError):
                test_gpui.validated_source(package)


if __name__ == '__main__':
    unittest.main()
