"""Validate the production source boundary used by the private AX test runner."""
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

from scripts import test_gpui


class TestSourceTests(unittest.TestCase):
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
