from pathlib import Path
import json
import unittest
import re

ROOT = Path(__file__).resolve().parents[1]
TAURI = ROOT / 'app/src-tauri'

class UpdaterFixturePolicy(unittest.TestCase):
    def test_fixture_trust_seams_are_feature_gated_and_identity_checked(self):
        source = (TAURI / 'src/commands/updater.rs').read_text()
        self.assertIn('#[cfg(any(feature = "shutdown-test-support", feature = "native-host-qa"))]\nmod fixture {', source)
        self.assertIn('#[cfg(any(feature = "shutdown-test-support", feature = "native-host-qa"))]\n    let updater = fixture::configure(app, updater)?;', source)
        self.assertIn('app.config().identifier == "com.localdictation.shutdown-test"', source)
        self.assertIn('if !enabled(app) { return Ok(updater); }', re.sub(r'\s+', ' ', source))
        core = (TAURI / 'crates/murmur-core/src/updater/mod.rs').read_text()
        self.assertIn('#[cfg(feature = "updater-test-support")]\n    pub fn isolated_fixture', core)
        self.assertIn('"com.localdictation.shutdown-test" | "com.localdictation.gpui-host-qa110"', re.sub(r'\s+', ' ', core))
        self.assertIn('cfg!(feature = "shutdown-test-support")', source)
        self.assertIn('cfg!(feature = "native-host-qa")', source)
        self.assertIn('app.config().identifier == "com.localdictation.gpui-host-qa110"', source)
        self.assertIn('Native updater fixture must use loopback HTTP', core)
        self.assertIn('take(65537)', source)
        self.assertNotIn('MURMUR_UPDATER_FIXTURE', core)
        self.assertIn('feed: FEED.into(), key: PUBLIC_KEY.into(), local_fixture: false', re.sub(r'\s+', ' ', core))

    def test_only_qa_config_grants_fixture_permission(self):
        config = json.loads((TAURI / 'tauri.shutdown-test.conf.json').read_text())
        self.assertEqual(config['identifier'], 'com.localdictation.shutdown-test')
        caps = [c for c in config['app']['security']['capabilities'] if isinstance(c, dict) and c['identifier'] == 'updater-test-actions']
        self.assertEqual(len(caps), 1)
        self.assertEqual(caps[0]['windows'], ['main'])
        self.assertEqual(caps[0]['permissions'], ['updater-test:allow-updater-test-run'])
        for path in (TAURI / 'capabilities').glob('*.json'):
            self.assertNotIn('updater-test:', path.read_text())
            self.assertNotIn('updater:default', path.read_text())
            self.assertNotIn('process:allow-restart', path.read_text())

    def test_installer_requires_every_packaged_executable(self):
        config = json.loads((TAURI / 'tauri.macos.conf.json').read_text())
        packaged = {'ui', *(Path(value).name for value in config['bundle']['externalBin'])}
        source = (TAURI / 'crates/murmur-core/src/updater/install.rs').read_text()
        required = re.search(r'const EXECUTABLES:.*?= &\[(.*?)\];', source, re.S)
        self.assertIsNotNone(required)
        self.assertEqual(set(re.findall(r'"([^"]+)"', required.group(1))), packaged)

    def test_production_trust_matches_installed_clients_and_has_no_override(self):
        config = json.loads((TAURI / 'tauri.conf.json').read_text())['plugins']['updater']
        source = (TAURI / 'crates/murmur-core/src/updater/feed.rs').read_text()
        self.assertIn(config['pubkey'], source)
        self.assertEqual(config['endpoints'], ['https://github.com/georgenijo/murmur-app/releases/latest/download/latest-v2.json'])
        self.assertIn(config['endpoints'][0], source)
        self.assertNotIn('minisign_verify', (TAURI / 'src/commands/updater.rs').read_text())
        for workflow in (ROOT / '.github/workflows').glob('*.yml'):
            self.assertNotIn('--features shutdown-test-support', workflow.read_text())

if __name__ == '__main__':
    unittest.main()
