import unittest
from check_restart_policy import violations, check_repository, method_allow_scopes, symbol_literal_allowances, check_clippy_config, REQUIRED_METHODS, TERMINATING_SYMBOLS
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

    def test_unrelated_exit_is_not_reserved(self):
        for source in ['fn exit(code: i32) -> i32 { code } let result = exit(37);', 'let gate = self.exit.clone();']:
            self.assertFalse(violations(source))
        self.assertTrue(violations('fn bypass() { std::process::exit(0); }'))

    def test_lint_allowances_are_exact_function_scopes(self):
        sanctioned = '#[allow(clippy::disallowed_methods)] fn terminal(app: &tauri::AppHandle) { app.exit(0); }'
        scopes, failures = method_allow_scopes(sanctioned, Path('app/src-tauri/src/shutdown.rs'))
        self.assertFalse(failures)
        self.assertEqual(len(scopes), 1)
        for source in [
            '#![allow(clippy::disallowed_methods)] fn terminal() {}',
            '#[allow(clippy::disallowed_methods)] mod terminal {}',
            '#[allow(clippy::disallowed_methods)] fn other() {}',
            '#[expect(clippy::disallowed_methods)] fn terminal() {}',
            '#[cfg_attr(test, allow(clippy::disallowed_methods))] fn terminal() {}',
            '#[allow(warnings)] fn terminal() {}',
            '#[allow(clippy::restriction)] fn terminal() {}',
            'mod bypass { #[allow(clippy::disallowed_methods)] fn terminal() {} }',
            'fn unrelated() { #[allow(clippy::disallowed_methods)] fn terminal() {} }',
        ]:
            with self.subTest(source=source):
                self.assertTrue(method_allow_scopes(source, Path('app/src-tauri/src/shutdown.rs'))[1])
        self.assertTrue(method_allow_scopes(sanctioned, Path('other.rs'))[1])

    def test_process_exit_allowance_does_not_spill_outside_function(self):
        source = '#[allow(clippy::disallowed_methods)] fn terminal() { std::process::exit(0); } fn bypass() { std::process::exit(0); }'
        scopes, _ = method_allow_scopes(source, Path('app/src-tauri/src/shutdown.rs'))
        failures = violations(source, method_scopes=scopes)
        self.assertEqual(len(failures), 1)
        self.assertGreater(failures[0], scopes[0][1])

    def test_clippy_config_cannot_be_overridden(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            self.assertTrue(check_clippy_config(root))
            (root / 'clippy.toml').write_text('disallowed-methods = ' + str(sorted(REQUIRED_METHODS)).replace("'", '"'))
            self.assertFalse(check_clippy_config(root))
            member = root / 'member'
            member.mkdir()
            (member / '.clippy.toml').write_text('disallowed-methods = []')
            self.assertTrue(check_clippy_config(root))

    def test_compiled_bypass_fixture_inventory(self):
        fixtures = Path(__file__).parent / 'fixtures/restart-clippy'
        for name in ['grouped_std_alias', 'grouped_libc_alias', 'grouped_libc_exit_alias', 'macro_forwarded_exit', 'direct_zero_outside_cli', 'apphandle_exit', 'apphandle_restart', 'abort_alias', 'libc_abort', 'plugin_init_alias', 'unrelated_exit']:
            self.assertTrue((fixtures / (name + '.txt')).is_file())

    def test_foreign_termination_declarations_and_link_attributes(self):
        for symbol in sorted(TERMINATING_SYMBOLS):
            for abi in ['', '"C"', '"C-unwind"', '"system"', '"stdcall"']:
                for source in [
                    f'unsafe extern {abi} {{ fn {symbol}(code: i32) -> !; }}',
                    f'unsafe extern {abi} {{ #[link_name = "{symbol}"] fn stop(code: i32) -> !; }}',
                    f'#[link(name = "{symbol}")] unsafe extern {abi} {{ fn stop(); }}',
                    f'unsafe extern {abi} {{ #[cfg_attr(target_os="macos", link_name=r#"{symbol}"#)] fn stop(); }}',
                ]:
                    with self.subTest(source=source):
                        self.assertTrue(violations(source))
        self.assertTrue(violations('extern "C" { #[link_name="\\x5fexit"] fn stop(); }'))
        self.assertTrue(violations('extern "C" { #[link_name=concat!("_e", "xit")] fn stop(); }'))
        self.assertTrue(violations('extern "C" { #[link_name=SYMBOL] fn stop(); }'))

    def test_benign_foreign_declarations_and_opaque_link_text(self):
        for source in [
            'unsafe extern "system" { fn puts(s: *const u8); fn getpid() -> i32; }',
            '#[link(name="System")] extern "C-unwind" { #[link_name="getpid"] fn pid() -> i32; }',
            'extern "C" { fn r#getpid(); }',
            'extern "C" fn exit(code: i32) -> i32 { code }',
            'let text = r##"extern "C" { fn _exit(); } #[link_name="abort"]"##;',
            '/* #[link(name="kill")] extern "C" { fn abort(); } */',
        ]:
            with self.subTest(source=source):
                self.assertFalse(violations(source))

    def test_macro_foreign_declarations_fail_closed(self):
        for delimiter, closing in [('{', '}'), ('(', ')'), ('[', ']')]:
            for body in [
                'unsafe extern "C" { fn $symbol(code: i32) -> !; }',
                'unsafe extern $abi { fn $symbol(code: i32) -> !; }',
                'unsafe extern $abi { fn getpid() -> i32; }',
                'unsafe extern $($abi)* { fn $symbol(code: i32) -> !; }',
                'unsafe extern "C" { fn getpid() -> i32; }',
            ]:
                source = (f'macro_rules! ffi {delimiter} ($symbol:ident, $abi:literal) => {{ {body} }}; {closing}'
                          ' ffi!(_exit, "C");')
                with self.subTest(source=source):
                    self.assertTrue(violations(source))
        # Metavariables also fail closed outside a macro_rules definition.
        self.assertTrue(violations('extern $abi { fn getpid(); }'))
        self.assertTrue(violations('extern "C" { fn $symbol(); }'))

    def test_exact_termination_symbol_literals(self):
        for symbol in sorted(TERMINATING_SYMBOLS):
            for prefix in ['', 'b', 'c', 'r', 'br', 'cr', 'r##', 'br##', 'cr##']:
                hashes = '#' * prefix.count('#')
                for suffix in ['', '\\0', '\\x00'] if 'r' not in prefix else ['', '\0']:
                    literal = f'{prefix}"{symbol}{suffix}"{hashes}'
                    with self.subTest(literal=literal):
                        source = f'let symbol = {literal};'
                        self.assertIn(source.index(prefix + '"'), violations(source))
        self.assertTrue(violations('let symbol = b"\\x5fexit\\0";'))
        self.assertTrue(violations('let symbol = c"\\u{5_f}exit";'))
        self.assertTrue(violations('let symbol = "_ex\\\n    it\\0";'))

    def test_benign_symbol_text_and_macros(self):
        for source in [
            'let values = ["exit_code", "abort_reason", "please exit now", "exit completed"];',
            'let values = [b"exit_code\\0", c"abort_reason", r#"log exit"#];',
            'let text = r"\\u{5f}exit";',
            'let text = "\\\\u{5f}exit";',
            'macro_rules! identity { ($symbol:ident) => { stringify!($symbol) }; } identity!(_exit);',
            'macro_rules! message { () => { "extern \\"C\\" { fn _exit(); }" }; }',
            '// "_exit" b"abort\\0" c"kill" r"raise"\n/* outer /* "exit" */ */',
        ]:
            with self.subTest(source=source):
                self.assertFalse(violations(source))

    def test_symbol_literals_are_checked_in_every_rust_source(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / 'nested/examples/symbols.rs'
            path.parent.mkdir(parents=True)
            path.write_text('fn main() { let _symbol = c"_exit"; }')
            self.assertTrue(check_repository(root))
            path.write_text('fn main() { let _message = "exit completed"; }')
            self.assertFalse(check_repository(root))

    def test_qa_action_exception_is_scoped_to_the_exact_match_arm(self):
        source = '''mod test_support {
            #[allow(clippy::disallowed_methods)]
            fn shutdown_test_action() {
                match action.as_str() { "exit" => app.exit(0), _ => () }
                let symbol = "exit";
            }
        }'''
        path = Path('app/src-tauri/src/shutdown.rs')
        allowances = symbol_literal_allowances(source, path)
        self.assertEqual(allowances, {source.index('"exit"')})
        self.assertEqual(violations(source, symbol_allowances=allowances), [source.rindex('"exit"')])
        for modified, relative in [
            (source, Path('other.rs')),
            (source.replace('shutdown_test_action', 'terminal'), path),
            (source.replace('mod test_support', 'mod other'), path),
            (source.replace('app.exit(0)', 'dlsym("exit")'), path),
        ]:
            with self.subTest(source=modified, relative=relative):
                self.assertFalse(symbol_literal_allowances(modified, relative))

    def test_foreign_fixture_inventory(self):
        fixtures = Path(__file__).parent / 'fixtures/restart-foreign'
        for name in ['macro_forwarded_name', 'macro_forwarded_abi', 'macro_fixed_extern',
                     'dlsym_c_string', 'dlsym_byte_string', 'dlsym_string',
                     'libloading_get', 'raw_symbol_strings', 'benign_strings', 'benign_macros']:
            self.assertTrue((fixtures / (name + '.txt')).is_file())
        for fixture in fixtures.glob('*.txt'):
            with self.subTest(fixture=fixture.name):
                self.assertEqual(not violations(fixture.read_text()), fixture.stem.startswith('benign_'))

if __name__ == '__main__':
    unittest.main()
