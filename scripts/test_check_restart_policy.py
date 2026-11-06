import unittest
from check_restart_policy import violations, check_repository
from pathlib import Path
import tempfile
class RestartPolicy(unittest.TestCase):
    def test_bypasses(self):
        for source in ['app.restart()', 'app.r#restart()', 'app.request_restart()', 'AppHandle::restart(&app)', 'Alias::<Wry>::restart(&app)', 'use tauri::AppHandle::restart as relaunch;', 'let f = AppHandle::request_restart;', 'macro_rules! bypass { () => { $app.restart() } }', 'app./* nested /* comment */ */restart()']:
            with self.subTest(source=source): self.assertTrue(violations(source))
    def test_plugin_and_special_exit_code(self):
        for source in ['app.plugin(tauri_plugin_process::init())', 'use tauri_plugin_process::init as original; app.plugin(original())', 'use tauri_plugin_process as proc; proc::init()', 'app.exit(tauri::RESTART_EXIT_CODE)', 'use tauri::RESTART_EXIT_CODE as code; app.exit(code)', 'macro_rules! bypass { () => { tauri_plugin_process::init() } }']:
            with self.subTest(source=source): self.assertTrue(violations(source))
    def test_opaque_rust(self):
        for source in ['// app.restart()\nlet x=1;', 'let s = r###"app.request_restart()"###;', '/* outer /* app.restart() */ end */', 'let c = \'r\';']:
            self.assertFalse(violations(source))
    def test_only_exact_adapter_call(self):
        self.assertFalse(violations('app.request_restart()', adapter=True))
        for source in ['other.request_restart()', 'let f=AppHandle::request_restart;', 'app.restart()', 'app.request_restart(); app.request_restart();', '']:
            self.assertTrue(violations(source, adapter=True))
    def test_compatibility_definition_is_not_an_api_permit(self):
        self.assertFalse(violations('fn restart() {} tauri::generate_handler![exit, restart]', plugin=True))
        self.assertTrue(violations('fn restart() { app.restart(); }', plugin=True))

    def test_exit_argument_allowlist(self):
        for expression in ['i32::MAX', 'std::i32::MAX', '2147483647', '2_147_483_647i32', '0x7fff_ffff', '0o17777777777', '0b1111111111111111111111111111111', 'CODE', '(0)', 'code()', '0 + CODE']:
            with self.subTest(expression=expression):
                self.assertTrue(violations(f'const CODE: i32 = i32::MAX; app.exit({expression});'))
        for source in ['let f = AppHandle::exit; f(&app, 2147483647);', 'AppHandle::exit(&app, i32::MAX)', 'macro_rules! bypass { ($code:expr) => { $app.exit($code) } }']:
            self.assertTrue(violations(source))
        self.assertFalse(violations('app.exit(/* ordinary coordinated quit */ 0)'))

    def test_renamed_cargo_package_in_every_manifest_table(self):
        for table in ['dependencies', 'dev-dependencies', 'build-dependencies', 'workspace.dependencies', 'target.\'cfg(target_os = "macos")\'.dependencies', 'patch.crates-io', 'patch.\'https://example.invalid/registry\'']:
            with self.subTest(table=table), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                member = root / 'members/nested'
                member.mkdir(parents=True)
                (member / 'Cargo.toml').write_text(f'[{table}]\noriginal-process = {{ package = "tauri-plugin-process", version = "2" }}\n')
                (member / 'lib.rs').write_text('builder.plugin(original_process::init());')
                self.assertTrue(check_repository(root))
                (member / 'lib.rs').write_text('use original_process as proc; proc::init();')
                self.assertTrue(check_repository(root))
                (member / 'lib.rs').write_text('builder.plugin(process_plugin::init());')
                self.assertFalse(check_repository(root))

    def test_workspace_inherited_rename(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'Cargo.toml').write_text('[workspace.dependencies]\noriginal = { package = "tauri-plugin-process", version = "2" }')
            member = root / 'member'
            member.mkdir()
            (member / 'Cargo.toml').write_text('[dependencies]\noriginal.workspace = true')
            (member / 'lib.rs').write_text('original::init();')
            self.assertTrue(check_repository(root))

    def test_bad_manifest_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'Cargo.toml').write_text('[dependencies')
            self.assertTrue(check_repository(root))
