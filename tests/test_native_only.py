import unittest
from scripts.check_native_only import manifest_violations

class NativeDependencyBoundary(unittest.TestCase):
    def test_rejects_aliases_optional_build_dev_and_target_edges(self):
        for section in ['dependencies', 'build-dependencies', 'dev-dependencies', 'target.\'cfg(windows)\'.dependencies']:
            for name in ['tauri', 'tauri-runtime-wry', 'wry', 'tao']:
                with self.subTest(section=section, name=name):
                    source = f'[{section}]\nhidden = {{package="{name}", version="1", optional=true}}\n'
                    self.assertEqual(manifest_violations(source), [name])
    def test_standalone_importer_and_vocabulary_are_not_runtime_dependencies(self):
        self.assertEqual(manifest_violations('[dependencies]\nobjc2-web-kit="0.3"\n# tauri vocabulary\n'), [])
