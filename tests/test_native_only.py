import unittest
from unittest import mock
from scripts import check_native_only
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

    def test_gpui_production_graph_rejects_runtimes_and_linked_importer(self):
        for banned in ['tauri', 'tao', 'wry', 'murmur-webkit', 'objc2-web-kit']:
            def output(command, **kwargs):
                if command[0] == 'git':
                    return ''
                package = command[command.index('-p') + 1]
                if package == 'murmur-ui':
                    self.assertIn('production-host', command)
                    self.assertEqual(command[command.index('--edges') + 1], 'normal')
                    self.assertEqual(command[command.index('--target') + 1], 'aarch64-apple-darwin')
                    return f'{package} v1\n{banned} v1\n'
                return f'{package} v1\n'
            with self.subTest(banned=banned), mock.patch.object(check_native_only.subprocess, 'check_output', side_effect=output):
                with self.assertRaises(SystemExit):
                    check_native_only.main()
