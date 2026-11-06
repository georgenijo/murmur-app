import unittest
from check_targeted_events import violations


class TargetedEventsGuardTests(unittest.TestCase):
    def test_broadcast_spellings_and_aliases(self):
        for source in ['app.emit("e", ());', 'app\n . emit::<()>("e", ());',
                       'tauri::Emitter::emit(&app, "e", ());',
                       'let broadcast = Emitter::emit;', 'app.emit_all("e", ());',
                       'AppHandle::emit_str(&app, "e", payload);',
                       'app.emit_filter("e", (), |_| true);']:
            with self.subTest(source=source):
                self.assertTrue(violations(source))

    def test_targeted_calls_callbacks_and_literal_examples(self):
        self.assertEqual(violations('''
            app.emit_to("main", "e", ()); emit(observation);
            // app.emit("e", ());
            /* nested /* app.emit("e", ()); */ comment */
            let example = r##"app.emit("e", ());"##;
            let normal = "app.emit(\\\"e\\\", ());";
        '''), [])
