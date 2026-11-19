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
//! * **Structure, on the production source of the spawners and of both entry callbacks**
//!   (D-S2c-16). Parsed with `syn`, so a formatting change cannot hide a defect and a comment
//!   cannot fake one, and counting only what a body **executes**: a call inside a closure or a
//!   nested `async` block that is built and never run proves nothing. The ticker spawner
//!   must execute `register()` and then spawn, on the async runtime, a task that itself
//!   awaits the ticker loop; the VAD spawner must skip exactly a recording that did not ask
//!   for streaming and a disabled VAD, and then execute `vad_stream::start`; each entry's
//!   ready callback, the last argument of the ready decision, must call both spawners with
//!   its own three arguments. `PLANTS` lists every way to drop the handoff that must fail:
//!   a spawner that registers the ticker and drops the task, a loop in a nested future nobody
//!   polls, an inverted VAD guard, and the rest.
//!
//! * **Pins (D-S2c-16 amendment 1).** The structural check cannot be complete against
//!   ordinary edits, so the normalised-token digest of the two ticker spawners, the VAD
//!   spawner and both ready callbacks is pinned, with `check_shadow_diff.py`'s own
//!   normalisation. Any edit fails `the_ticker_and_vad_starting_code_is_the_pinned_code`;
//!   a re-pin needs a reviewed justification that the ticker and VAD still start for every
//!   eligible recording. `CONTROLS` are the mutations that must break their pin. The
//!   structural check stays as documentation and an extra guard.
//!
//! The spawners themselves run only in the lifecycle cutover PR's runtime check (D-S2c-16:
//! a cutover blocker).
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

/// An executed call: its callee, its line, whether the call is `.await`ed, and the call.
type Executed<'a> = (String, usize, bool, &'a syn::ExprCall);

/// The calls a body **executes**, in source order. It does not descend into a closure, a
/// nested `async` block or a nested item: such a body is created, and runs only if
/// something calls or polls it, so a call found there proves nothing about this body. (A
/// loop moved into a future that is built and dropped must not count.) Start it on the
/// statements of the body, or on an `async` block's own block, to look inside that one.
struct Exec<'a>(Vec<Executed<'a>>);

impl<'ast> Visit<'ast> for Exec<'ast> {
    fn visit_expr_closure(&mut self, _: &'ast syn::ExprClosure) {}
    fn visit_expr_async(&mut self, _: &'ast syn::ExprAsync) {}
    fn visit_item(&mut self, _: &'ast syn::Item) {}
    fn visit_expr_await(&mut self, awaited: &'ast syn::ExprAwait) {
        if let syn::Expr::Call(call) = &*awaited.base {
            self.0
                .push((callee(call), call.span().start().line, true, call));
            // The call's arguments are executed too; the call itself was recorded here.
            for argument in &call.args {
                self.visit_expr(argument);
            }
            return;
        }
        syn::visit::visit_expr_await(self, awaited);
    }
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        self.0
            .push((callee(call), call.span().start().line, false, call));
        syn::visit::visit_expr_call(self, call);
    }
}

fn executed_in_fn(function: &syn::ItemFn) -> Vec<Executed<'_>> {
    let mut exec = Exec(Vec::new());
    exec.visit_block(&function.block);
    exec.0
}

fn executed_in_block(block: &syn::Block) -> Vec<Executed<'_>> {
    let mut exec = Exec(Vec::new());
    exec.visit_block(block);
    exec.0
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

/// `name` or `name.clone()`.
fn is_path_or_clone(expr: &syn::Expr, name: &str) -> bool {
    is_path(expr, name)
        || matches!(expr, syn::Expr::MethodCall(call)
            if call.method == "clone" && call.args.is_empty() && is_path(&call.receiver, name))
}

/// The name a parameter pattern binds.
fn bound_name(pattern: &syn::Pat) -> Option<String> {
    match pattern {
        syn::Pat::Ident(ident) => Some(ident.ident.to_string()),
        _ => None,
    }
}

/// The name of the `index`th parameter of a function.
fn parameter(function: &syn::ItemFn, index: usize) -> Option<String> {
    match function.sig.inputs.iter().nth(index)? {
        syn::FnArg::Typed(typed) => bound_name(&typed.pat),
        syn::FnArg::Receiver(_) => None,
    }
}

/// The leaves of an `a || b || c` chain.
fn or_terms(expr: &syn::Expr) -> Vec<&syn::Expr> {
    match expr {
        syn::Expr::Binary(binary) if matches!(binary.op, syn::BinOp::Or(_)) => {
            let mut terms = or_terms(&binary.left);
            terms.extend(or_terms(&binary.right));
            terms
        }
        syn::Expr::Paren(paren) => or_terms(&paren.expr),
        other => vec![other],
    }
}

/// `!inner`, with the inner expression.
fn negated(expr: &syn::Expr) -> Option<&syn::Expr> {
    match expr {
        syn::Expr::Unary(unary) if matches!(unary.op, syn::UnOp::Not(_)) => Some(&unary.expr),
        syn::Expr::Paren(paren) => negated(&paren.expr),
        _ => None,
    }
}

/// `<anything>.<field>`.
fn is_field(expr: &syn::Expr, field: &str) -> bool {
    matches!(expr, syn::Expr::Field(access)
        if matches!(&access.member, syn::Member::Named(name) if name == field))
}

/// The top-level `if <cond> { return; }` statements of a function: the early returns.
fn early_returns(function: &syn::ItemFn) -> Vec<(&syn::Expr, usize)> {
    function
        .block
        .stmts
        .iter()
        .filter_map(|statement| match statement {
            syn::Stmt::Expr(syn::Expr::If(branch), _) if branch.else_branch.is_none() => {
                let only_return = matches!(branch.then_branch.stmts.as_slice(),
                    [syn::Stmt::Expr(syn::Expr::Return(r), _)] if r.expr.is_none());
                only_return.then(|| (&*branch.cond, branch.span().start().line))
            }
            _ => None,
        })
        .collect()
}

/// What is wrong with the production handoff in `source` (`recording.rs`), as messages;
/// empty when the handoff holds.
fn handoff_defects(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).expect("recording.rs parses");
    let mut defects = Vec::new();

    // The ticker spawner, in each of its `cfg` variants: it executes `register()`, and after
    // it executes the spawn, on the async runtime, of a task whose own body **awaits** the
    // ticker loop. A task that is built and dropped (a future nobody polls), or a loop
    // moved into a nested future, is not a handoff.
    let tickers = functions(&file, "spawn_dictation_partial_ticker");
    if tickers.len() < 2 {
        defects.push(format!(
            "expected the ticker spawner's two cfg variants, found {}",
            tickers.len()
        ));
    }
    for (index, spawner) in tickers.iter().enumerate() {
        let register_name = parameter(spawner, 2).unwrap_or_default();
        let calls = executed_in_fn(spawner);
        let register = calls
            .iter()
            .find(|(path, _, _, call)| *path == register_name && call.args.is_empty())
            .map(|(_, line, _, _)| *line);
        let spawned = calls.iter().find_map(|(path, line, _, call)| {
            if !path.ends_with("async_runtime::spawn") {
                return None;
            }
            let Some(syn::Expr::Async(task)) = call.args.first() else {
                return None;
            };
            executed_in_block(&task.block)
                .iter()
                .any(|(inner, _, awaited, _)| inner == "run_partial_ticker_loop" && *awaited)
                .then_some(*line)
        });
        match (register, spawned) {
            (None, _) => defects.push(format!(
                "ticker spawner #{index} never executes `register()`: nothing registers the ticker"
            )),
            (Some(_), None) => defects.push(format!(
                "ticker spawner #{index} registers the ticker but never spawns, on the async \
                 runtime, a task that itself awaits `run_partial_ticker_loop`: the task is \
                 dropped, or the loop is in a future nobody polls"
            )),
            (Some(register), Some(spawned)) if spawned < register => defects.push(format!(
                "ticker spawner #{index} spawns the task before `register()` decides"
            )),
            _ => {}
        }
    }

    // The VAD spawner skips only a recording that did not ask for streaming, a disabled VAD
    // or a missing model, and otherwise executes `vad_stream::start` for the recording.
    let vads = functions(&file, "spawn_vad_stream");
    if vads.len() != 1 {
        defects.push(format!(
            "expected one `spawn_vad_stream`, found {}",
            vads.len()
        ));
    }
    for spawner in vads {
        let (app, id) = (
            parameter(spawner, 0).unwrap_or_default(),
            parameter(spawner, 1).unwrap_or_default(),
        );
        let guards: Vec<(Vec<&syn::Expr>, usize)> = early_returns(spawner)
            .into_iter()
            .map(|(cond, line)| (or_terms(cond), line))
            .collect();
        let skips = |wanted: &dyn Fn(&syn::Expr) -> bool| {
            guards
                .iter()
                .any(|(terms, _)| terms.iter().any(|term| negated(term).is_some_and(wanted)))
        };
        if !skips(&|inner| is_field(inner, "streaming_pipeline_enabled")) {
            defects.push(
                "`spawn_vad_stream` does not skip exactly a recording that did not ask for \
                 streaming (`if !<settings>.streaming_pipeline_enabled .. { return; }`)"
                    .to_string(),
            );
        }
        if !skips(&|inner| {
            matches!(inner, syn::Expr::Call(call)
                if callee(call).ends_with("vad::is_enabled")
                    && call.args.first().is_some_and(|a| is_field(a, "vad_sensitivity")))
        }) {
            defects.push(
                "`spawn_vad_stream` does not skip exactly a disabled VAD \
                 (`!vad::is_enabled(<settings>.vad_sensitivity)`)"
                    .to_string(),
            );
        }
        let guard_end = guards.iter().map(|(_, line)| *line).max().unwrap_or(0);
        let starts = executed_in_fn(spawner).iter().any(|(path, line, _, call)| {
            path.ends_with("vad_stream::start")
                && *line > guard_end
                && call.args.len() == 3
                && is_path(&call.args[0], &app)
                && is_path(&call.args[1], &id)
        });
        if !starts {
            defects.push(
                "`spawn_vad_stream` never executes `vad_stream::start(app, recording_id, ..)` \
                 after its guards"
                    .to_string(),
            );
        }
    }

    // Both ready entry points hand the ready decision a callback, and that callback, with
    // its own three arguments (the handle, the recording id and `register`), starts the VAD
    // stream and hands `register` to the ticker spawner.
    for (entry, receiver) in [
        ("handle_audio_lifecycle", "handle_audio_lifecycle_events"),
        (
            "publish_recording_ready_if_audio_ready",
            "publish_recording_ready_with",
        ),
    ] {
        let functions = functions(&file, entry);
        if functions.len() != 1 {
            defects.push(format!("expected one `{entry}`, found {}", functions.len()));
            continue;
        }
        let executed = executed_in_fn(functions[0]);
        let callback = executed
            .iter()
            .find(|(path, _, _, _)| path == receiver)
            .and_then(|(_, _, _, call)| call.args.last());
        let Some(syn::Expr::Closure(closure)) = callback else {
            defects.push(format!(
                "`{entry}` does not pass a closure as the last argument of `{receiver}`"
            ));
            continue;
        };
        let names: Vec<String> = closure.inputs.iter().filter_map(bound_name).collect();
        let [handle, id, register] = names.as_slice() else {
            defects.push(format!(
                "`{entry}`'s ready callback does not bind three named arguments"
            ));
            continue;
        };
        let mut exec = Exec(Vec::new());
        exec.visit_expr(&closure.body);
        let vad = exec.0.iter().any(|(path, _, _, call)| {
            path == "spawn_vad_stream"
                && call.args.len() == 2
                && is_path_or_clone(&call.args[0], handle)
                && is_path(&call.args[1], id)
        });
        let ticker = exec.0.iter().any(|(path, _, _, call)| {
            path == "spawn_dictation_partial_ticker"
                && call.args.len() == 3
                && is_path_or_clone(&call.args[0], handle)
                && is_path(&call.args[1], id)
                && is_path(&call.args[2], register)
        });
        if !vad {
            defects.push(format!(
                "`{entry}`'s ready callback does not execute `spawn_vad_stream({handle}, {id})` \
                 with its own arguments"
            ));
        }
        if !ticker {
            defects.push(format!(
                "`{entry}`'s ready callback does not execute \
                 `spawn_dictation_partial_ticker({handle}, {id}, {register})` with its own arguments"
            ));
        }
    }
    defects
}

// ------------------------------------------------------------------------ the pins

/// The normalised-token digests of the production code that starts the ticker and the VAD,
/// pinned (D-S2c-16 amendment 1). The structural check above cannot be complete against
/// ordinary edits (an `if whisper` around the spawn, an inverted `register()` guard, an
/// inverted model-file check, a start inside `if false` each passed it), so **any** edit to
/// these fails here, and a re-pin needs a reviewed justification that the ticker and the
/// VAD still start for every eligible recording. The digest is `check_shadow_diff.py`'s own
/// (its tokenizer, its trailing-comma normalisation and its `pair_hash`, called from the
/// test), so comments, whitespace and rustfmt's commas do not move it, and nothing else does.
const PINS: [(&str, &str); 5] = [
    (
        "the ticker spawner, default build (`spawn_dictation_partial_ticker`, cfg(not(feature)))",
        "0c896bdc613ea45a",
    ),
    (
        "the ticker spawner, dictation-shadow build (`spawn_dictation_partial_ticker`, cfg(feature))",
        "ad397eed99fc8e72",
    ),
    ("the VAD spawner (`spawn_vad_stream`)", "916425f266e46c7e"),
    (
        "the bridge `Ready` ready callback (`handle_audio_lifecycle`)",
        "f9e709a6da9034c2",
    ),
    (
        "the start command's R104.2 ready callback (`publish_recording_ready_if_audio_ready`)",
        "d562ab70fc542119",
    ),
];

/// The source text of a node between two `proc_macro2` locations (1-based line, 0-based
/// character column).
fn text_between(
    source: &str,
    start: proc_macro2::LineColumn,
    end: proc_macro2::LineColumn,
) -> String {
    let offset = |at: proc_macro2::LineColumn| -> usize {
        let line_start: usize = source
            .split_inclusive('\n')
            .take(at.line - 1)
            .map(str::len)
            .sum();
        let line = source[line_start..].split('\n').next().unwrap_or("");
        line_start
            + line
                .char_indices()
                .nth(at.column)
                .map_or(line.len(), |(byte, _)| byte)
    };
    source[offset(start)..offset(end)].to_string()
}

/// The texts of the five pinned items of `source`, in [`PINS`] order.
fn pinned_texts(source: &str) -> Vec<String> {
    let file = syn::parse_file(source).expect("recording.rs parses");
    let function_text = |function: &syn::ItemFn| {
        let start = function.attrs.first().map_or_else(
            || function.span().start(),
            |attribute| attribute.span().start(),
        );
        text_between(source, start, function.span().end())
    };
    let is_default_build = |function: &syn::ItemFn| {
        function.attrs.iter().any(|attribute| {
            attribute.path().is_ident("cfg")
                && attribute
                    .meta
                    .require_list()
                    .is_ok_and(|list| list.tokens.to_string().starts_with("not"))
        })
    };
    let tickers = functions(&file, "spawn_dictation_partial_ticker");
    let default_build: Vec<_> = tickers.iter().filter(|f| is_default_build(f)).collect();
    let feature_build: Vec<_> = tickers.iter().filter(|f| !is_default_build(f)).collect();
    let vads = functions(&file, "spawn_vad_stream");
    let callback = |entry: &str, receiver: &str| -> String {
        let functions = functions(&file, entry);
        assert_eq!(functions.len(), 1, "one `{entry}`");
        let executed = executed_in_fn(functions[0]);
        let callback = executed
            .iter()
            .find(|(path, _, _, _)| path == receiver)
            .and_then(|(_, _, _, call)| call.args.last());
        let Some(syn::Expr::Closure(closure)) = callback else {
            return format!("NO-CALLBACK {entry}");
        };
        text_between(source, closure.span().start(), closure.span().end())
    };
    // A missing item is its own text, so the pin fails on it instead of the test panicking.
    let one = |found: Vec<String>, name: &str| -> String {
        match found.as_slice() {
            [only] => only.clone(),
            other => format!("{} `{name}` items found", other.len()),
        }
    };
    vec![
        one(
            default_build.iter().map(|f| function_text(f)).collect(),
            "default-build ticker spawner",
        ),
        one(
            feature_build.iter().map(|f| function_text(f)).collect(),
            "feature-build ticker spawner",
        ),
        one(
            vads.iter().map(|f| function_text(f)).collect(),
            "VAD spawner",
        ),
        callback("handle_audio_lifecycle", "handle_audio_lifecycle_events"),
        callback(
            "publish_recording_ready_if_audio_ready",
            "publish_recording_ready_with",
        ),
    ]
}

/// `check_shadow_diff.py`'s normalised-token digest of each text, computed by that script's
/// own `tokenize`, `normalised` and `pair_hash` (the frozen-pair digest, with an empty twin).
/// A Python 3 that cannot be run fails the test: the pins fail closed.
fn normalised_digests(texts: &[String]) -> Vec<String> {
    use std::io::Write;
    const PROGRAM: &str = "import sys, json, importlib.util\n\
        spec = importlib.util.spec_from_file_location('check_shadow_diff', sys.argv[1])\n\
        module = importlib.util.module_from_spec(spec)\n\
        spec.loader.exec_module(module)\n\
        texts = json.load(sys.stdin)\n\
        json.dump([module.pair_hash(module.normalised([t for t, _ in module.tokenize(x)]), []) \
        for x in texts], sys.stdout)\n";
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/check_shadow_diff.py");
    let mut child = std::process::Command::new("python3")
        .args(["-I", "-c", PROGRAM])
        .arg(script)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("python3 runs check_shadow_diff.py's normaliser");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(serde_json::to_string(texts).expect("texts").as_bytes())
        .expect("the texts reach the normaliser");
    let output = child.wait_with_output().expect("the normaliser finishes");
    assert!(
        output.status.success(),
        "check_shadow_diff.py's normaliser failed"
    );
    serde_json::from_slice(&output.stdout).expect("the digests")
}

/// The pins `source` breaks, as messages that name the pin and the re-pin rule.
fn pin_defects(source: &str) -> Vec<String> {
    let digests = normalised_digests(&pinned_texts(source));
    PINS.iter()
        .zip(digests)
        .filter(|((_, pinned), digest)| digest != pinned)
        .map(|((name, pinned), digest)| {
            format!(
                "S8 pin: {name} changed (digest {digest}, pinned {pinned}). Re-pin only with a \
                 reviewed justification that the ticker and the VAD still start for every \
                 eligible recording (D-S2c-16 amendment 1)."
            )
        })
        .collect()
}

/// The production ticker and VAD code is exactly the code a reviewer pinned, and each pin
/// is on the text it names (not on a failed lookup).
#[test]
fn the_ticker_and_vad_starting_code_is_the_pinned_code() {
    let texts = pinned_texts(&production_source());
    for (text, needles) in texts.iter().zip([
        &[
            "fn spawn_dictation_partial_ticker",
            "run_partial_ticker_loop",
            "not(feature",
        ][..],
        &[
            "fn spawn_dictation_partial_ticker",
            "run_partial_ticker_loop",
            "ticker_start_check",
        ],
        &["fn spawn_vad_stream", "vad_stream::start", "vad_model_path"],
        &[
            "spawn_vad_stream",
            "spawn_dictation_partial_ticker",
            "register",
        ],
        &[
            "spawn_vad_stream",
            "spawn_dictation_partial_ticker",
            "register",
        ],
    ]) {
        for needle in needles {
            assert!(
                text.contains(needle),
                "a pinned text is not the item it names (missing `{needle}`): {text}"
            );
        }
    }
    let defects = pin_defects(&production_source());
    assert!(defects.is_empty(), "{defects:#?}");
}

/// A control: a change to the production source, with the pins it must break (by a fragment
/// of the pin's name).
type Control = (
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static [&'static str],
);

/// Sol's round-2 mutations, which the structural check let through, and the callbacks'
/// own. Each is a source mutation of the real `recording.rs`, and each must fail by its pin.
const CONTROLS: &[Control] = &[
    (
        "the ticker spawn behind `if whisper` (an eligible Core ML recording loses its ticker)",
        &[
            (
                "    drop(tauri::async_runtime::spawn(async move {\n        let window_app",
                "    if whisper { drop(tauri::async_runtime::spawn(async move {\n        let window_app",
            ),
            (
                "        .await;\n    }));\n}",
                "        .await;\n    })); }\n}",
            ),
        ],
        &["default build", "dictation-shadow build"],
    ),
    (
        "an unconditional return before the ticker spawn",
        &[(
            "    let preview: WhisperPreview = Arc::new(Mutex::new(None));\n",
            "    let preview: WhisperPreview = Arc::new(Mutex::new(None));\n    return;\n",
        )],
        &["default build", "dictation-shadow build"],
    ),
    (
        "an inverted `register()` guard",
        &[("    if !register() {", "    if register() {")],
        &["default build", "dictation-shadow build"],
    ),
    (
        "an inverted VAD model-file check (VAD suppressed when the model is installed)",
        &[(
            "        || !vad::vad_model_path().is_some_and(|path| path.exists())",
            "        || vad::vad_model_path().is_some_and(|path| path.exists())",
        )],
        &["the VAD spawner"],
    ),
    (
        "`vad_stream::start` inside `if false`",
        &[(
            "    crate::vad_stream::start(app, recording_id, dictation_partial_is_current);\n}",
            "    if false {\n        crate::vad_stream::start(app, recording_id, dictation_partial_is_current);\n    }\n}",
        )],
        &["the VAD spawner"],
    ),
    (
        "the bridge's ready callback starts the ticker only `if false`",
        &[(
            "            spawn_dictation_partial_ticker(app_handle, recording_id, register);",
            "            if false { spawn_dictation_partial_ticker(app_handle, recording_id, register); }",
        )],
        &["the bridge `Ready` ready callback"],
    ),
    (
        "the start command's ready callback starts the VAD only `if false`",
        &[(
            "            spawn_vad_stream(app_handle.clone(), recording_id);\n            spawn_dictation_partial_ticker(app_handle.clone(), recording_id, register);",
            "            if false { spawn_vad_stream(app_handle.clone(), recording_id); }\n            spawn_dictation_partial_ticker(app_handle.clone(), recording_id, register);",
        )],
        &["R104.2 ready callback"],
    ),
];

/// Every control fails by the pin it names (and by no pin it does not), with the re-pin rule.
#[test]
fn every_control_mutation_fails_its_pin() {
    let source = production_source();
    for (name, replacements, expected) in CONTROLS {
        let planted = replacements
            .iter()
            .fold(source.clone(), |text, (from, to)| mutated(&text, from, to));
        let defects = pin_defects(&planted);
        assert_eq!(
            defects.len(),
            expected.len(),
            "control `{name}` must break exactly {expected:?}: {defects:#?}"
        );
        for fragment in *expected {
            assert!(
                defects
                    .iter()
                    .any(|defect| defect.contains(fragment)
                        && defect.contains("reviewed justification")),
                "control `{name}` did not break the pin for `{fragment}`: {defects:#?}"
            );
        }
    }
}

/// The digest is `check_shadow_diff.py`'s normalisation: comments, whitespace and a trailing
/// comma do not move it, a changed token does.
#[test]
fn the_pin_digest_is_the_shadow_diff_normalisation() {
    let base = "fn f(a: u8) { if a > 1 { g(a, 2) } }".to_string();
    let reflowed = "fn f(a: u8) {\n    // why\n    if a > 1 {\n        g(\n            a,\n            2,\n        )\n    }\n}"
        .to_string();
    let changed = "fn f(a: u8) { if a < 1 { g(a, 2) } }".to_string();
    let digests = normalised_digests(&[base, reflowed, changed]);
    assert_eq!(
        digests[0], digests[1],
        "formatting and comments move the pin"
    );
    assert_ne!(
        digests[0], digests[2],
        "a changed token does not move the pin"
    );
    assert_eq!(digests[0].len(), 16);
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

/// Every plant: a change to the production source that drops the handoff, with the defect
/// the structural check must report. `(name, [(from, to)], expected defect)`; each `from` is
/// replaced wherever it occurs (the ticker spawner has two `cfg` variants).
/// A plant: its name, its `(from, to)` replacements and the defect it must be reported as.
type Plant = (
    &'static str,
    &'static [(&'static str, &'static str)],
    &'static str,
);

const PLANTS: &[Plant] = &[
    (
        "registers the ticker and drops the task",
        &[("drop(tauri::async_runtime::spawn(async move {\n        let window_app", "drop((async move {\n        let window_app")],
        "registers the ticker but never spawns",
    ),
    (
        "the loop moved into a nested future that is created and never polled",
        &[("        run_partial_ticker_loop(", "        let _unpolled = async move { run_partial_ticker_loop("), ("            || crate::commands::dictation_preview::hide_on_ticker_exit(&app, recording_id),\n        )\n        .await;\n    }));", "            || crate::commands::dictation_preview::hide_on_ticker_exit(&app, recording_id),\n        )\n        .await };\n    }));")],
        "registers the ticker but never spawns",
    ),
    (
        "the loop is built but never awaited",
        &[("            || crate::commands::dictation_preview::hide_on_ticker_exit(&app, recording_id),\n        )\n        .await;\n    }));", "            || crate::commands::dictation_preview::hide_on_ticker_exit(&app, recording_id),\n        );\n    }));")],
        "registers the ticker but never spawns",
    ),
    (
        "the ticker spawner never registers",
        &[("    if !register() {\n        return;\n    }\n    // Decode while speaking", "    // Decode while speaking")],
        "never executes `register()`",
    ),
    (
        "the VAD spawner never starts the stream",
        &[("    crate::vad_stream::start(app, recording_id, dictation_partial_is_current);\n}", "    let _ = (app, recording_id);\n}")],
        "never executes `vad_stream::start",
    ),
    (
        "the VAD streaming-enabled guard is inverted",
        &[("    if !transcription.streaming_pipeline_enabled", "    if transcription.streaming_pipeline_enabled")],
        "does not skip exactly a recording that did not ask for streaming",
    ),
    (
        "the VAD enabled guard is inverted",
        &[("        || !vad::is_enabled(transcription.vad_sensitivity)", "        || vad::is_enabled(transcription.vad_sensitivity)")],
        "does not skip exactly a disabled VAD",
    ),
    (
        "the bridge's ready callback drops the ticker call",
        &[("            spawn_vad_stream(app_handle.clone(), recording_id);\n            spawn_dictation_partial_ticker(app_handle, recording_id, register);", "            spawn_vad_stream(app_handle.clone(), recording_id);\n            let _ = (app_handle, register);")],
        "`handle_audio_lifecycle`'s ready callback does not execute `spawn_dictation_partial_ticker",
    ),
    (
        "the start command's ready callback drops the VAD call",
        &[("            spawn_vad_stream(app_handle.clone(), recording_id);\n            spawn_dictation_partial_ticker(app_handle.clone(), recording_id, register);", "            spawn_dictation_partial_ticker(app_handle.clone(), recording_id, register);")],
        "`publish_recording_ready_if_audio_ready`'s ready callback does not execute `spawn_vad_stream",
    ),
    (
        "the bridge's ready callback hands the ticker another recording id",
        &[("            spawn_dictation_partial_ticker(app_handle, recording_id, register);", "            spawn_dictation_partial_ticker(app_handle, 0, register);")],
        "`handle_audio_lifecycle`'s ready callback does not execute `spawn_dictation_partial_ticker",
    ),
    (
        "the start command's ready callback hands the ticker its own register, not the decision's",
        &[("            spawn_dictation_partial_ticker(app_handle.clone(), recording_id, register);", "            spawn_dictation_partial_ticker(app_handle.clone(), recording_id, Box::new(|| true));")],
        "`publish_recording_ready_if_audio_ready`'s ready callback does not execute `spawn_dictation_partial_ticker",
    ),
    (
        "the bridge passes the ready decision a different callback",
        &[("        |app_handle, recording_id, register| {\n            spawn_vad_stream(app_handle.clone(), recording_id);\n            spawn_dictation_partial_ticker(app_handle, recording_id, register);\n        },\n    );", "        |_, _, _| {},\n    );\n    let _unused = |app_handle: tauri::AppHandle, recording_id: u64, register: Box<dyn FnOnce() -> bool>| {\n            spawn_vad_stream(app_handle.clone(), recording_id);\n            spawn_dictation_partial_ticker(app_handle, recording_id, register);\n        };")],
        "`handle_audio_lifecycle`'s ready callback does not bind three named arguments",
    ),
];

/// Every plant fails the structural check, with the defect it names. The plants include the
/// one the brief names (a spawner that registers the ticker and drops the task) and the
/// ones Sol found missed in the first round: a loop in a nested future that is never
/// polled, and an inverted VAD streaming-enabled guard.
#[test]
fn every_plant_that_drops_the_handoff_fails() {
    let source = production_source();
    for (name, replacements, expected) in PLANTS {
        let planted = replacements
            .iter()
            .fold(source.clone(), |text, (from, to)| mutated(&text, from, to));
        let defects = handoff_defects(&planted);
        assert!(
            defects.iter().any(|defect| defect.contains(expected)),
            "plant `{name}` was not caught as `{expected}`: {defects:#?}"
        );
    }
}
