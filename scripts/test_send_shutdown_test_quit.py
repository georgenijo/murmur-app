from pathlib import Path
import plistlib
import tempfile
import unittest
from send_shutdown_test_quit import validate_identity, IDENTIFIER
class SenderIdentity(unittest.TestCase):
    def test_only_matching_isolated_executable_is_admitted(self):
        with tempfile.TemporaryDirectory() as root:
            bundle = Path(root) / 'Synthetic.app'
            (bundle / 'Contents/MacOS').mkdir(parents=True)
            info = bundle / 'Contents/Info.plist'
            info.write_bytes(plistlib.dumps({'CFBundleIdentifier': IDENTIFIER, 'CFBundleExecutable': 'synthetic'}))
            binary = bundle / 'Contents/MacOS/synthetic'
            self.assertEqual(validate_identity(bundle, 12345, binary), binary)
            for pid, path in [(1, binary), (12345, '/tmp/other-process')]:
                with self.assertRaises(ValueError): validate_identity(bundle, pid, path)
            info.write_bytes(plistlib.dumps({'CFBundleIdentifier': 'other', 'CFBundleExecutable': 'synthetic'}))
            with self.assertRaises(ValueError): validate_identity(bundle, 12345, binary)

    def test_fixture_permission_is_main_only_and_absent_from_production(self):
        import json
        root = Path(__file__).resolve().parents[1] / 'app/src-tauri'
        fixture = json.loads((root / 'tauri.shutdown-test.conf.json').read_text())
        self.assertEqual(fixture['identifier'], IDENTIFIER)
        capabilities = fixture['app']['security']['capabilities']
        permission = [cap for cap in capabilities if isinstance(cap, dict)]
        self.assertEqual(len(permission), 1)
        self.assertEqual(permission[0]['windows'], ['main'])
        self.assertEqual(permission[0]['permissions'], ['shutdown-test:allow-shutdown-test-action'])
        for path in (root / 'capabilities').glob('*.json'):
            self.assertNotIn('shutdown-test:', path.read_text())
