import tempfile
from pathlib import Path
import unittest

from check_targeted_events import check_repository, violations


class TargetedEventsGuardTests(unittest.TestCase):
    def test_reserved_tokens_in_every_context(self):
        fixtures = [
            'app.emit("e", ());',
            'app\n . emit::<()>("e", ());',
            'use tauri::*;',
            'use tauri::{Manager, nested::*};',
            'use tauri::Emitter as E;',
            'use tauri::{Emitter as E, Manager};',
            'Emitter::emit(&app, "e", ());',
            '<AppHandle as Emitter>::emit(&app, "e", ());',
            'let broadcast = tauri::Emitter::emit;',
            'app.r#emit("event", ());',
            'app.emit_filter("event", (), |_| true);',
            'app.emit_str("event", payload);',
            'app.emit_str_filter("event", payload, |_| true);',
            'app.emit_to(EventTarget::AnyLabel { label }, "event", ());',
            '#[cfg(any())] fn disabled() { app.emit("event", ()); }',
            '''use tauri::*;
               macro_rules! forward {
                   ($app:expr, $($method:tt)*) => { ($app).$($method)*("event", ()) }
               }
               fn f(app: AppHandle) { forward!(app, emit); }''',
            # The method token alone is forbidden; no reliance on its expansion.
            'forward!(app, emit);',
            'forward!(app, r#emit_to);',
            'let emit_future_api = transport;',
            'fn emit() {}',
            'app.emit_all("event", ());',
        ]
        for source in fixtures:
            with self.subTest(source=source):
                self.assertTrue(violations(source))

    def test_comments_strings_and_unrelated_identifiers(self):
        self.assertEqual(violations('''
            publish(observation);
            let emitters = 1;
            // app.emit("e", ()); use tauri::*;
            /* nested /* app.emit("e", ()); */ comment */
            let example = r##"app.emit("e", ()); use tauri::*;"##;
            let bytes = br#"emit_filter"#;
            let c_string = c"emit_str";
            let normal = "app.emit(\\\"e\\\", ());";
            let event_name = "emit_status";
            let char = 'e';
        '''), [])

    def test_facades_and_crate_aliases_cannot_hide_tauri_globs(self):
        for source in [
            'use tauri as transport; use transport::*;',
            'use tauri::{self as transport}; use transport::*;',
            'use {tauri as transport};',
            'pub use tauri; use facade::tauri::*;',
            'pub(crate) use tauri::Manager;',
            'mod facade { pub use tauri::{self}; } use facade::tauri::*;',
            'extern crate tauri as transport; use transport::*;',
            'pub use ::tauri::{AppHandle};',
        ]:
            with self.subTest(source=source):
                self.assertTrue(violations(source))
        self.assertEqual(violations('use tauri::{Manager, AppHandle}; use super::*;'), [])

    def test_adapter_constrains_every_target_and_broadcast_token(self):
        for method in ['emit_to', 'emit_str_to']:
            self.assertEqual(violations(f'app.{method}(tauri::EventTarget::webview_window(label), "event", payload);', adapter=True), [])
            for target in ['tauri::EventTarget::Any', 'tauri::EventTarget::AnyLabel { label }',
                           'tauri::EventTarget::any()', 'tauri::EventTarget::labeled(label)',
                           'tauri::EventTarget::App', 'label', 'target', '"main"']:
                with self.subTest(method=method, target=target):
                    self.assertTrue(violations(f'app.{method}({target}, "event", payload);', adapter=True))
        for source in [
            'use tauri::EventTarget::{AnyLabel as Broad};',
            'let transport = Emitter::emit_to;',
            'app.r#emit("event", ());',
            'app.emit_filter("event", (), |_| true);',
            'app.emit_str("event", payload);',
            'app.emit_str_filter("event", payload, |_| true);',
            '($app).$($method)*("event", ());',
        ]:
            with self.subTest(source=source):
                self.assertTrue(violations(source, adapter=True))

    def test_every_repository_rust_source_is_scanned_even_outside_app(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'src').mkdir()
            (root / 'src/lib.rs').write_text('#[path = "../native_events.rs"] mod native;')
            (root / 'native_events.rs').write_text('fn f() { forward!(app, emit); }')
            failures = check_repository(root)
            self.assertEqual(len(failures), 1)
            self.assertTrue(failures[0].startswith('native_events.rs:'))

    def test_path_escape_outside_repository_and_excluded_sources(self):
        with tempfile.TemporaryDirectory() as directory:
            outer = Path(directory)
            root = outer / 'repo'
            (root / 'src').mkdir(parents=True)
            (root / 'target').mkdir()
            (root / 'target/events.rs').write_text('app.emit("event", ());')
            (outer / 'native_events.rs').write_text('app.emit("event", ());')
            for source in [
                '#[path = "../../native_events.rs"] mod native;',
                '#[path = r"../../native_events.rs"] mod native;',
                '#[path = "../target/events.rs"] mod native;',
                'include!("../../native_events.rs");',
                'include_str!(r#"../../native_events.rs"#);',
                'include_bytes!("../../native_events.rs");',
                'include!(concat!(env!("OUT_DIR"), "/events.rs"));',
                'include_str!(concat!(env!("OUT_DIR"), "/events.rs"));',
                '#[cfg(any())] #[path = "../../native_events.rs"] mod native;',
                '#[cfg_attr(any(), path = "../../native_events.rs")] mod native;',
                '#[r#path = "../../native_events.rs"] mod native;',
                'mod inner { #[path = "../../native_events.rs"] mod native; }',
                'r#include!("../../native_events.rs");',
            ]:
                with self.subTest(source=source):
                    (root / 'src/lib.rs').write_text(source)
                    self.assertTrue(check_repository(root))

    def test_local_inclusions_and_non_rust_fixtures_are_allowed(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'src').mkdir()
            (root / 'src/module.rs').write_text('fn publish() {}')
            (root / 'src/lib.rs').write_text('''
                #[path = /* comment */ "module.rs"] mod local;
                include!(r#"module.rs"#);
                include_str!("module.rs",);
                include_str!("../../fixture.json");
                // #[path = "../../escape.rs"] mod disabled;
                let text = "include!(\\\"../../escape.rs\\\")";
            ''')
            self.assertEqual(check_repository(root), [])

    def test_symlink_source_cannot_escape_scanned_roots(self):
        with tempfile.TemporaryDirectory() as directory:
            outer = Path(directory)
            root = outer / 'repo'
            root.mkdir()
            (outer / 'events.rs').write_text('app.emit("event", ());')
            (root / 'lib.rs').symlink_to(outer / 'events.rs')
            self.assertTrue(check_repository(root))
            (root / 'lib.rs').unlink()
            (root / 'native').symlink_to(outer, target_is_directory=True)
            self.assertTrue(check_repository(root))

    def test_token_pasting_input_is_still_reserved(self):
        # Stable macro tokens cannot smuggle in a complete reserved identifier.
        # Arbitrary proc-macro synthesis is outside this source guard's remit.
        for source in ['forward! { [emit] }', 'let r#emit_future = callback;',
                       'use ::tauri::{r#Emitter as E};',
                       '#[cfg(any())] use tauri::*;']:
            with self.subTest(source=source):
                self.assertTrue(violations(source))
