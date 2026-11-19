//! A positive check of the ready side-task handoff (D-S2c-15 S8, Sol 4).
//!
//! Both ready entry points hand a recording's side tasks to the production spawners: the
//! lifecycle bridge's `Ready` (`handle_audio_lifecycle`) and the start command's R104.2
//! (`publish_recording_ready_if_audio_ready`) each run `spawn_vad_stream` and
//! `spawn_dictation_partial_ticker(.., register)`. The spawners need a Wry `AppHandle`, which
//! no test can build (the golden harness runs on the mock runtime and supplies its own
//! spawn closure), so the handoff has two halves here:
//!
//! * **Runtime, at both entry points, through the real ports and decisions.** A supported
//!   model hands the spawn closure the recording, whose `register` passes the production
//!   ticker gate; a genuine skip (an unsupported model) hands it too, and the spawner returns
//!   before `register`: nothing is registered.
//! * **Structure, on the production source of the spawners and of both entry closures.**
//!   Parsed with `syn`, so a formatting change cannot hide a defect and a comment cannot
//!   fake one. The ticker spawner must run `register()` and then spawn the task that runs
//!   the loop; the VAD spawner must start the stream; each entry closure must call both
//!   with `register` handed through. A spawner that registers the ticker and drops the
//!   task is a failing plant (`a_spawner_that_registers_and_drops_the_task_fails`).
#![cfg(test)]

use super::golden_harness::*;
use super::golden_tests::{recording, sub_from};
use super::*;
use syn::spanned::Spanned;
use syn::visit::Visit;

// ------------------------------------------------------------------ runtime half

const REGISTERED: &str = "SD ticker 1 registered=true";

/// How many times the entry handed the pair to its spawner, and how many tickers the
/// spawner then registered.
fn handoff(world: &Arc<World>, spawns: usize) -> (usize, usize) {
    let registered = world
        .trace()
        .iter()
        .filter(|line| line.starts_with("SD ticker "))
        .count();
    (spawns, registered)
}

fn set_context_model(world: &Arc<World>, model: &str) {
    world.install_context(1);
    let state = world.state();
    let mut context = state
        .app_state
        .active_context(1)
        .expect("the fixture installed a context");
    Arc::make_mut(&mut context).transcription.model_name = model.to_string();
    state.app_state.set_active_context(1, context);
}

/// The bridge's `Ready` for a recording whose context is installed.
fn bridge_ready(world: &Arc<World>, model: &str) {
    set_context_model(world, model);
    world.notify(1, AudioLifecycleEvent::Ready);
}

/// The start command: its cold start reaches R104.2 once the context is installed, with
/// the model the setting names.
fn start_command(world: &Arc<World>, model: &str) {
    world
        .state()
        .app_state
        .dictation
        .lock_or_recover()
        .model_name = model.to_string();
    recording(world);
}

fn supported_through_the_bridge(world: &Arc<World>) {
    bridge_ready(world, "base.en");
    let (spawns, registered) = handoff(world, world.bridge_side_task_spawns());
    assert_eq!(
        spawns, 1,
        "the bridge's Ready handed the pair to the spawner"
    );
    assert_eq!(registered, 1, "the supported model's ticker was registered");
    assert!(world.trace().contains(&REGISTERED.to_string()));
}

fn skipped_through_the_bridge(world: &Arc<World>) {
    bridge_ready(world, "small.en");
    let (spawns, registered) = handoff(world, world.bridge_side_task_spawns());
    assert_eq!(
        spawns, 1,
        "the bridge's Ready handed the pair to the spawner"
    );
    assert_eq!(
        registered, 0,
        "an unsupported model spawns nothing: the spawner returns before `register`"
    );
}

fn supported_through_the_start_command(world: &Arc<World>) {
    start_command(world, "base.en");
    let (spawns, registered) = handoff(world, world.side_task_spawns());
    assert!(spawns >= 1, "R104.2 handed the pair to the spawner");
    assert_eq!(
        registered, 1,
        "the supported model's ticker was registered once, at R104.2"
    );
    assert!(world.trace().contains(&REGISTERED.to_string()));
}

fn skipped_through_the_start_command(world: &Arc<World>) {
    start_command(world, "small.en");
    let (spawns, registered) = handoff(world, world.side_task_spawns());
    assert!(spawns >= 1, "R104.2 handed the pair to the spawner");
    assert_eq!(
        registered, 0,
        "an unsupported model spawns nothing: the spawner returns before `register`"
    );
}

#[test]
fn a_supported_model_hands_its_ticker_off_at_both_ready_entry_points() {
    for (title, status, id, body) in [
        (
            "handoff: a supported model through the bridge's Ready",
            DictationStatus::Starting,
            1,
            supported_through_the_bridge as fn(&Arc<World>),
        ),
        (
            "handoff: a supported model through the start command (R104.2)",
            DictationStatus::Idle,
            0,
            supported_through_the_start_command as fn(&Arc<World>),
        ),
    ] {
        super::pd_witness_tests::repeat(&sub_from(title, status, id, body));
    }
}

#[test]
fn a_genuine_skip_spawns_nothing_at_both_ready_entry_points() {
    for (title, status, id, body) in [
        (
            "handoff: an unsupported model through the bridge's Ready",
            DictationStatus::Starting,
            1,
            skipped_through_the_bridge as fn(&Arc<World>),
        ),
        (
            "handoff: an unsupported model through the start command (R104.2)",
            DictationStatus::Idle,
            0,
            skipped_through_the_start_command as fn(&Arc<World>),
        ),
    ] {
        super::pd_witness_tests::repeat(&sub_from(title, status, id, body));
    }
}

/// With the feature, the production spawner's own check names the skip reason, so a skip
/// is a genuine one for the right reason: the production `ticker_start_check` on the
/// recorded context, the same function the spawner and the harness's spawner run.
#[cfg(feature = "dictation-shadow")]
#[test]
fn the_skip_reason_is_the_production_checks() {
    use crate::dictation_shadow::types::TickerSkipReason;
    let app_state = AppState::default();
    assert!(matches!(
        ticker_start_check(&app_state, 1),
        Err((TickerSkipReason::NoContext, None))
    ));
    for (model, supported) in [("base.en", true), ("small.en", false)] {
        let destination = select_recording_destination(
            &crate::frontmost::DeliveryTargetSnapshot::Incomplete,
            crate::smart_destination::Snapshot::Disabled,
        );
        let mut context = resolve_recording_context(&app_state, destination);
        Arc::make_mut(&mut context).transcription.model_name = model.to_string();
        app_state.set_active_context(1, context);
        match ticker_start_check(&app_state, 1) {
            Ok(_) => assert!(supported, "{model} is spawned"),
            Err((reason, held)) => {
                assert!(!supported, "{model} is skipped");
                assert_eq!(reason, TickerSkipReason::NoLivePreview);
                assert!(held.is_some(), "the skip holds the recorded context");
            }
        }
    }
}

// ----------------------------------------------------------------- structural half

/// The path of a call's callee, as `a::b::c`.
fn callee(call: &syn::ExprCall) -> String {
    match &*call.func {
        syn::Expr::Path(path) => path
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect::<Vec<_>>()
            .join("::"),
        _ => String::new(),
    }
}

/// Every call in a body, in source order: `(callee, line, the call)`.
struct Calls<'a>(Vec<(String, usize, &'a syn::ExprCall)>);

impl<'ast> Visit<'ast> for Calls<'ast> {
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        self.0.push((callee(call), call.span().start().line, call));
        syn::visit::visit_expr_call(self, call);
    }
}

fn calls_in<'a>(node: &'a impl CallsIn) -> Vec<(String, usize, &'a syn::ExprCall)> {
    let mut calls = Calls(Vec::new());
    node.collect(&mut calls);
    calls.0
}

trait CallsIn {
    fn collect<'a>(&'a self, calls: &mut Calls<'a>);
}

impl CallsIn for syn::ItemFn {
    fn collect<'a>(&'a self, calls: &mut Calls<'a>) {
        calls.visit_item_fn(self);
    }
}

impl CallsIn for syn::Expr {
    fn collect<'a>(&'a self, calls: &mut Calls<'a>) {
        calls.visit_expr(self);
    }
}

/// The functions named `name` in the file, at any module depth, in every `cfg` variant.
fn functions<'a>(file: &'a syn::File, name: &str) -> Vec<&'a syn::ItemFn> {
    struct Find<'a, 'n> {
        name: &'n str,
        found: Vec<&'a syn::ItemFn>,
    }
    impl<'ast> Visit<'ast> for Find<'ast, '_> {
        fn visit_item_fn(&mut self, function: &'ast syn::ItemFn) {
            if function.sig.ident == self.name {
                self.found.push(function);
            }
            syn::visit::visit_item_fn(self, function);
        }
    }
    let mut find = Find {
        name,
        found: Vec::new(),
    };
    find.visit_file(file);
    find.found
}

/// Whether `expr` is the bare path `name`.
fn is_path(expr: &syn::Expr, name: &str) -> bool {
    matches!(expr, syn::Expr::Path(path) if path.path.is_ident(name))
}

/// What is wrong with the production handoff in `source` (`recording.rs`), as messages;
/// empty when the handoff holds.
fn handoff_defects(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).expect("recording.rs parses");
    let mut defects = Vec::new();

    // The ticker spawner, in each of its `cfg` variants: it runs `register()`, and after it
    // spawns, on the async runtime, the task that runs the ticker loop. A task that is
    // built and dropped (a future nobody polls) is not a handoff.
    let tickers = functions(&file, "spawn_dictation_partial_ticker");
    if tickers.len() < 2 {
        defects.push(format!(
            "expected the ticker spawner's two cfg variants, found {}",
            tickers.len()
        ));
    }
    for (index, spawner) in tickers.iter().enumerate() {
        let calls = calls_in(*spawner);
        let register = calls
            .iter()
            .find(|(path, _, call)| path == "register" && call.args.is_empty())
            .map(|(_, line, _)| *line);
        let spawned = calls.iter().find_map(|(path, line, call)| {
            let on_runtime = path.ends_with("async_runtime::spawn");
            let task = call.args.first()?;
            let runs_the_loop = matches!(task, syn::Expr::Async(_))
                && calls_in(task)
                    .iter()
                    .any(|(inner, _, _)| inner == "run_partial_ticker_loop");
            (on_runtime && runs_the_loop).then_some(*line)
        });
        match (register, spawned) {
            (None, _) => defects.push(format!(
                "ticker spawner #{index} never calls `register()`: nothing registers the ticker"
            )),
            (Some(_), None) => defects.push(format!(
                "ticker spawner #{index} registers the ticker but never spawns the task that runs \
                 `run_partial_ticker_loop` on the async runtime: the task is dropped"
            )),
            (Some(register), Some(spawned)) if spawned < register => defects.push(format!(
                "ticker spawner #{index} spawns the task before `register()` decides"
            )),
            _ => {}
        }
    }

    // The VAD spawner starts the incremental stream for the recording.
    let vads = functions(&file, "spawn_vad_stream");
    if vads.len() != 1 {
        defects.push(format!(
            "expected one `spawn_vad_stream`, found {}",
            vads.len()
        ));
    }
    for spawner in vads {
        let starts = calls_in(spawner).iter().any(|(path, _, call)| {
            path.ends_with("vad_stream::start")
                && call.args.len() == 3
                && is_path(&call.args[0], "app")
                && is_path(&call.args[1], "recording_id")
        });
        if !starts {
            defects.push(
                "`spawn_vad_stream` never starts `vad_stream::start(app, recording_id, ..)`"
                    .to_string(),
            );
        }
    }

    // Both ready entry points hand `register` to the ticker spawner and start the VAD
    // stream from the one closure they pass the ready decision.
    for entry in [
        "handle_audio_lifecycle",
        "publish_recording_ready_if_audio_ready",
    ] {
        let functions = functions(&file, entry);
        if functions.len() != 1 {
            defects.push(format!("expected one `{entry}`, found {}", functions.len()));
            continue;
        }
        struct Closures<'a>(Vec<&'a syn::ExprClosure>);
        impl<'ast> Visit<'ast> for Closures<'ast> {
            fn visit_expr_closure(&mut self, closure: &'ast syn::ExprClosure) {
                self.0.push(closure);
                syn::visit::visit_expr_closure(self, closure);
            }
        }
        let mut closures = Closures(Vec::new());
        closures.visit_item_fn(functions[0]);
        let hands_off = closures.0.iter().any(|closure| {
            let calls = calls_in(&*closure.body);
            let vad = calls.iter().any(|(path, _, _)| path == "spawn_vad_stream");
            let ticker = calls.iter().any(|(path, _, call)| {
                path == "spawn_dictation_partial_ticker"
                    && call.args.len() == 3
                    && is_path(&call.args[2], "register")
            });
            vad && ticker
        });
        if !hands_off {
            defects.push(format!(
                "`{entry}` has no closure that calls `spawn_vad_stream` and \
                 `spawn_dictation_partial_ticker(.., register)`"
            ));
        }
    }
    defects
}

fn production_source() -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/recording.rs");
    std::fs::read_to_string(path).expect("recording.rs")
}

/// The handoff the production source has today: both spawners hand their tasks off and both
/// entries call them with `register`.
#[test]
fn the_production_spawners_and_both_entries_hand_their_tasks_off() {
    let defects = handoff_defects(&production_source());
    assert!(defects.is_empty(), "{defects:#?}");
}

/// Replace every occurrence of `from` in `source` (the ticker spawner has two `cfg`
/// variants, which share most of their text), and insist it is there.
fn mutated(source: &str, from: &str, to: &str) -> String {
    assert!(
        source.contains(from),
        "the plant's anchor is not in recording.rs: {from}"
    );
    source.replace(from, to)
}

/// The plant the brief names: a production spawner that registers the ticker and drops the
/// task. The task is built and never spawned, in both `cfg` variants.
#[test]
fn a_spawner_that_registers_and_drops_the_task_fails() {
    let source = production_source();
    let planted = mutated(
        &source,
        "drop(tauri::async_runtime::spawn(async move {\n        let window_app",
        "drop((async move {\n        let window_app",
    );
    let defects = handoff_defects(&planted);
    assert!(
        defects
            .iter()
            .any(|defect| defect.contains("registers the ticker but never spawns the task")),
        "{defects:#?}"
    );
}

/// A spawner whose task no longer runs the ticker loop, a spawner that never registers, a
/// VAD spawner that never starts the stream and an entry that forgets to hand `register` on
/// each fail too.
#[test]
fn every_other_way_to_drop_the_handoff_fails_too() {
    let source = production_source();
    for (from, to, expected) in [
        (
            "        run_partial_ticker_loop(\n            interval,",
            "        std::future::ready(()).await;\n        let _ = (\n            interval,",
            "never spawns the task",
        ),
        (
            "    if !register() {\n        return;\n    }\n    // Decode while speaking",
            "    // Decode while speaking",
            "never calls `register()`",
        ),
        (
            "    crate::vad_stream::start(app, recording_id, dictation_partial_is_current);\n}",
            "    let _ = (app, recording_id);\n}",
            "never starts `vad_stream::start",
        ),
        (
            "            spawn_vad_stream(app_handle.clone(), recording_id);\n            spawn_dictation_partial_ticker(app_handle, recording_id, register);",
            "            spawn_vad_stream(app_handle.clone(), recording_id);\n            let _ = (app_handle, register);",
            "`handle_audio_lifecycle` has no closure",
        ),
        (
            "            spawn_vad_stream(app_handle.clone(), recording_id);\n            spawn_dictation_partial_ticker(app_handle.clone(), recording_id, register);",
            "            spawn_dictation_partial_ticker(app_handle.clone(), recording_id, register);",
            "`publish_recording_ready_if_audio_ready` has no closure",
        ),
    ] {
        let defects = handoff_defects(&mutated(&source, from, to));
        assert!(
            defects.iter().any(|defect| defect.contains(expected)),
            "plant `{expected}` was not caught: {defects:#?}"
        );
    }
}
