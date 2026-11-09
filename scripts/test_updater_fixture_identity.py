"""The OTA fixture generator must serve exactly the bundle identities that the
app's fixture gate honors, or a native update downloads but cannot install."""
import json
from pathlib import Path
import plistlib
import re
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(Path(__file__).resolve().parent))
import updater_test_fixture as fixture  # noqa: E402

QA = 'com.localdictation.gpui-host-qa110'


def bundle(root: Path, identifier: str, version: str = '0.62.2', executables=fixture.EXECUTABLES) -> Path:
    app = root / 'Murmur.app'
    (app / 'Contents/MacOS').mkdir(parents=True)
    (app / 'Contents/Info.plist').write_bytes(plistlib.dumps({
        'CFBundleIdentifier': identifier,
        'CFBundleExecutable': 'ui',
        'CFBundleShortVersionString': version,
    }))
    for name in executables:
        (app / 'Contents/MacOS' / name).write_bytes(b'synthetic')
    return app


class FixtureIdentity(unittest.TestCase):
    def serve(self, identifier, **kwargs):
        with tempfile.TemporaryDirectory() as tmp:
            return fixture.bundle_version(bundle(Path(tmp), identifier, **kwargs))

    def test_gpui_qa_identity_is_servable_beside_the_legacy_one(self):
        self.assertEqual(self.serve(QA, version='0.62.2'), '0.62.2')
        self.assertEqual(self.serve('com.localdictation.shutdown-test'), '0.62.2')

    def test_production_and_unrelated_identities_are_rejected(self):
        production = json.loads((ROOT / 'app/macos/bundle.json').read_text())['identifier']
        for identifier in (production, 'com.localdictation', 'com.localdictation.gpui-host-qa111', 'com.example.other'):
            with self.subTest(identifier=identifier), self.assertRaises(ValueError):
                self.serve(identifier)
        with tempfile.TemporaryDirectory() as tmp:
            app = bundle(Path(tmp), production)
            with self.assertRaises(ValueError):
                fixture.prepare(app, Path.home() / '.cache' / 'never-created-fixture', 18990)

    def test_a_bundle_the_installer_would_refuse_is_not_served(self):
        with self.assertRaises(ValueError):
            self.serve(QA, executables=('ui',))

    def test_generator_identities_match_the_app_fixture_gates(self):
        services = (ROOT / 'app/crates/murmur-services/src/commands/updater.rs').read_text()
        enabled = set(re.findall(r'config\(\)\.identifier == "([^"]+)"', services))
        core = (ROOT / 'app/crates/murmur-core/src/updater/mod.rs').read_text()
        gate = re.search(r'self\.identifier\.as_str\(\),\s*((?:"[^"]+"\s*\|?\s*)+)\)', core)
        self.assertIsNotNone(gate)
        accepted = set(re.findall(r'"([^"]+)"', gate.group(1)))
        self.assertIn(QA, enabled)
        self.assertEqual(set(fixture.IDENTIFIERS), accepted)
        self.assertLessEqual(enabled, set(fixture.IDENTIFIERS))

    def test_executables_match_the_installer_requirement(self):
        install = (ROOT / 'app/crates/murmur-core/src/updater/install.rs').read_text()
        required = re.search(r'const EXECUTABLES:.*?= &\[(.*?)\];', install, re.S)
        self.assertIsNotNone(required)
        self.assertEqual(set(re.findall(r'"([^"]+)"', required.group(1))), set(fixture.EXECUTABLES))


if __name__ == '__main__':
    unittest.main()
