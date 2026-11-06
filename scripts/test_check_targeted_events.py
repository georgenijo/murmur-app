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
            emit(observation);
            // app.emit("e", ());
            /* nested /* app.emit("e", ()); */ comment */
            let example = r##"app.emit("e", ());"##;
            let normal = "app.emit(\\\"e\\\", ());";
        '''), [])

    def test_all_transport_bypasses(self):
        for source in [
            'app.emit_to(tauri::EventTarget::Any, "event", ());',
            'app.emit_to(tauri::EventTarget::AnyLabel { label: "main".into() }, "event", ());',
            'app.emit_to(EventTarget::any(), "event", ());',
            'app.emit_to(EventTarget::labeled("main"), "event", ());',
            'app.emit_to("main", "event", ());',
            'app.r#emit("event", ());',
            'app.r#emit_filter("event", (), |_| true);',
            'app.emit_str("event", payload);',
            'use tauri::Emitter as Transport;',
            'use tauri::{Emitter as Transport, Manager};',
            'macro_rules! forward { ($app:expr, $method:ident) => { ($app).$method("event", ()) } } forward!(app, emit);',
            'macro_rules! forward { ($app:expr, $method:ident) => { ($app).$method("event", ()) } } forward!(app, emit_to);',
            'macro_rules! qualified { ($method:ident) => { Transport::$method } }',
        ]:
            with self.subTest(source=source):
                self.assertTrue(violations(source))

    def test_adapter_constrains_every_target(self):
        for method in ['emit_to', 'emit_str_to']:
            self.assertEqual(violations(f'app.{method}(tauri::EventTarget::webview_window(label), "event", payload);', adapter=True), [])
            for target in ['tauri::EventTarget::Any', 'tauri::EventTarget::AnyLabel { label }',
                           'tauri::EventTarget::any()', 'tauri::EventTarget::labeled(label)',
                           'tauri::EventTarget::App', 'label', 'target', '"main"']:
                with self.subTest(method=method, target=target):
                    self.assertTrue(violations(f'app.{method}({target}, "event", payload);', adapter=True))
        self.assertTrue(violations('let transport = Emitter::emit_to;', adapter=True))
        self.assertTrue(violations('app.r#emit("event", ());', adapter=True))
        self.assertTrue(violations('($app).$method("event", ());', adapter=True))
