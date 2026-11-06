import unittest
from check_restart_policy import violations
class RestartPolicy(unittest.TestCase):
    def test_bypasses(self):
        for source in ['app.restart()', 'app.r#restart()', 'app.request_restart()', 'AppHandle::restart(&app)', 'Alias::<Wry>::restart(&app)', 'use tauri::AppHandle::restart as relaunch;', 'let f = AppHandle::request_restart;', 'macro_rules! bypass { () => { $app.restart() } }', 'app./* nested /* comment */ */restart()']:
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
