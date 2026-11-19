//! A positive check of the ready side-task handoff (D-S2c-15 S8, Sol 4).
//!
//! Both ready entry points hand a recording's side tasks to the production spawners: the
//! lifecycle bridge's `Ready` (`handle_audio_lifecycle`) and the start command's R104.2
//! (`publish_recording_ready_if_audio_ready`) each run `spawn_vad_stream` and
//! `spawn_dictation_partial_ticker(.., register)`. Since the lifecycle cutover the spawners
//! (and the window helpers they reach) are generic over the Tauri runtime, so the handoff has
//! two halves here:
//!
//! * **Runtime, at both entry points, through the real ports, decisions and the real
//!   spawners** (`World::run_real_spawners`, the `real_spawners` knob, on the mock runtime).
//!   An eligible recording (a model that supports a live preview, streaming and VAD on, a VAD
//!   model file at the path the spawner checks) is registered once by the production ticker
//!   spawner's own `register` call and reaches `vad_stream::start` once; a genuine skip (an
//!   unsupported model, streaming off) makes the spawners return before `register` and before
//!   `start`: nothing is registered and no stream starts.
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
//!   eligible recording. Every pinned target, and each callback's receiver, must be found
//!   **exactly once**: zero or several (a `cfg` split, a second function) fail closed and
//!   say which. `CONTROLS` are the mutations that must break their pin. The
//!   structural check stays as documentation and an extra guard.
//!
//! **Re-pin at the lifecycle cutover (D-S2c-16 amendment 1 c).** Making the three spawners
//! generic over the runtime changed their signatures and nothing else: the ticker spawner (both
//! `cfg` variants) and `spawn_vad_stream` gained `<R: tauri::Runtime>` and take
//! `AppHandle<R>`; every statement of every body, the `register()` guard, the ticker task and
//! the `vad_stream::start` call are token-for-token what they were, and the two ready callbacks
//! (pinned unchanged) still call both spawners with `register` passed through. The ticker and
//! the VAD therefore still start for every eligible recording, which the runtime half above now
//! shows by running the real spawners at both entry points.
#![cfg(test)]

use super::golden_harness::*;
use super::golden_tests::{recording, sub_from};
use super::*;
use syn::spanned::Spanned;
use syn::visit::Visit;

// ------------------------------------------------------------------ runtime half

const REGISTERED: &str = "SD ticker 1 registered=true";

/// What the entry handed the **production** spawners, and what they did with it.
struct Handoff {
    /// Times the entry handed the pair to its spawn closure.
    spawns: usize,
    /// Tickers the production ticker spawner registered (its `register` call).
    registered: usize,
    /// Times the production VAD spawner reached `vad_stream::start`.
    vad_starts: usize,
}

/// Run both entry points through the real `spawn_vad_stream` and
/// `spawn_dictation_partial_ticker` (generic over the runtime since the lifecycle cutover),
/// on the mock runtime. `eligible` installs a recorded context the spawners accept: a model
/// that supports a live preview, streaming on, VAD on, and a VAD model file at the path the
/// spawner checks. Otherwise the model does not support a live preview and streaming is off,
/// so both spawners return before they register or start anything.
struct Case {
    eligible: bool,
    /// Keeps the VAD model path pointed at a directory that holds (or lacks) the file.
    _vad_dir: tempfile::TempDir,
    _vad: crate::vad::test_seam::Installed,
    vad_starts_before: usize,
}

impl Case {
    fn new(world: &Arc<World>, eligible: bool) -> Self {
        let dir = tempfile::tempdir().expect("a directory for the VAD model path");
        if eligible {
            std::fs::write(dir.path().join(crate::vad::VAD_MODEL_FILENAME), b"vad")
                .expect("the VAD model file the spawner checks");
        }
        let vad = crate::vad::test_seam::install(dir.path());
        world.knobs().real_spawners = true;
        {
            let state = world.state();
            let mut dictation = state.app_state.dictation.lock_or_recover();
            dictation.model_name = Self::model(eligible).to_string();
            dictation.streaming_pipeline_enabled = eligible;
            dictation.vad_sensitivity = 50;
        }
        Self {
            eligible,
            _vad_dir: dir,
            _vad: vad,
            vad_starts_before: crate::vad_stream::test_starts::count(1),
        }
    }

    fn model(eligible: bool) -> &'static str {
        if eligible {
            "base.en"
        } else {
            "small.en"
        }
    }

    /// The handoff the entry made, after `spawns` times through its spawn closure.
    fn handoff(&self, world: &Arc<World>, spawns: usize) -> Handoff {
        // Retire the recording so the ticker task the spawner started (it is detached and
        // runs after the test) ends at its first check instead of ticking.
        world.state().app_state.cancel_recording(1);
        Handoff {
            spawns,
            registered: world
                .trace()
                .iter()
                .filter(|line| line.starts_with("SD ticker "))
                .count(),
            vad_starts: crate::vad_stream::test_starts::count(1) - self.vad_starts_before,
        }
    }

    fn assert_handed_off(&self, handoff: &Handoff, entry: &str) {
        assert!(
            handoff.spawns >= 1,
            "{entry} handed the pair to the spawners"
        );
        if self.eligible {
            assert_eq!(
                handoff.registered, 1,
                "{entry}: the production ticker spawner registered the supported model's ticker once"
            );
            assert_eq!(
                handoff.vad_starts, 1,
                "{entry}: the production VAD spawner started the stream once"
            );
        } else {
            assert_eq!(
                handoff.registered, 0,
                "{entry}: an unsupported model spawns no ticker: the spawner returns before `register`"
            );
            assert_eq!(
                handoff.vad_starts, 0,
                "{entry}: streaming off spawns no VAD stream: the spawner returns before `start`"
            );
        }
    }
}

/// The bridge's `Ready` for a recording whose context is installed with the case's model.
fn bridge_ready(world: &Arc<World>, case: &Case) {
    world.install_context(1);
    let state = world.state();
    let mut context = state
        .app_state
        .active_context(1)
        .expect("the fixture installed a context");
    {
        let transcription = &mut Arc::make_mut(&mut context).transcription;
        transcription.model_name = Case::model(case.eligible).to_string();
        transcription.streaming_pipeline_enabled = case.eligible;
        transcription.vad_sensitivity = 50;
    }
    state.app_state.set_active_context(1, context);
    world.notify(1, AudioLifecycleEvent::Ready);
}

fn through_the_bridge(world: &Arc<World>, eligible: bool) {
    let case = Case::new(world, eligible);
    bridge_ready(world, &case);
    let handoff = case.handoff(world, world.bridge_side_task_spawns());
    assert_eq!(handoff.spawns, 1, "the bridge's Ready handed the pair once");
    case.assert_handed_off(&handoff, "the bridge's Ready");
    if eligible {
        assert!(world.trace().contains(&REGISTERED.to_string()));
    }
}

/// The start command: its cold start reaches R104.2 once the context is installed, with
/// the settings the case names.
fn through_the_start_command(world: &Arc<World>, eligible: bool) {
    let case = Case::new(world, eligible);
    recording(world);
    let handoff = case.handoff(world, world.side_task_spawns());
    case.assert_handed_off(&handoff, "the start command's R104.2");
    if eligible {
        assert!(world.trace().contains(&REGISTERED.to_string()));
    }
}

fn supported_through_the_bridge(world: &Arc<World>) {
    through_the_bridge(world, true);
}

fn skipped_through_the_bridge(world: &Arc<World>) {
    through_the_bridge(world, false);
}

fn supported_through_the_start_command(world: &Arc<World>) {
    through_the_start_command(world, true);
}

fn skipped_through_the_start_command(world: &Arc<World>) {
    through_the_start_command(world, false);
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

/// The skip is the right one: with the real spawners, each skip reason is the spawner's own
/// check on the recorded context, not the harness's.
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

/// The final segment of a callee path: `x`, `self::x`, `super::x`, `Self::x` and
/// `crate::a::b::x` are all `x`, so a qualified spelling of a receiver or a spawner counts
/// as that receiver or spawner and cannot hide a second one. (An alias import, a macro or a
/// function pointer is deliberate evasion and out of scope, D-S2c-11 (b).)
fn last_segment(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
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

/// The ready callback of `entry`: the last argument of its call to `receiver`. **Exactly
/// one** `entry` function and exactly one call to `receiver` in it, or the error says which
/// count was wrong: a second branch (a `cfg` split, say) is a second callback nobody
/// pinned, so it fails here rather than being resolved to the first one in source order.
fn ready_callback<'a>(
    file: &'a syn::File,
    entry: &str,
    receiver: &str,
) -> Result<&'a syn::ExprClosure, String> {
    let entries = functions(file, entry);
    let [function] = entries.as_slice() else {
        return Err(format!(
            "expected exactly one `{entry}`, found {}",
            entries.len()
        ));
    };
    let executed = executed_in_fn(function);
    let calls: Vec<_> = executed
        .iter()
        .filter(|(path, _, _, _)| last_segment(path) == receiver)
        .collect();
    let [(_, _, _, call)] = calls.as_slice() else {
        return Err(format!(
            "expected exactly one call to `{receiver}` in `{entry}`, found {}",
            calls.len()
        ));
    };
    match call.args.last() {
        Some(syn::Expr::Closure(closure)) => Ok(closure),
        _ => Err(format!(
            "`{entry}` does not pass a closure as the last argument of `{receiver}`"
        )),
    }
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
    if tickers.len() != 2 {
        defects.push(format!(
            "expected exactly the ticker spawner's two cfg variants, found {}",
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
                .any(|(inner, _, awaited, _)| {
                    last_segment(inner) == "run_partial_ticker_loop" && *awaited
                })
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
        let closure = match ready_callback(&file, entry, receiver) {
            Ok(closure) => closure,
            Err(defect) => {
                defects.push(defect);
                continue;
            }
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
            last_segment(path) == "spawn_vad_stream"
                && call.args.len() == 2
                && is_path_or_clone(&call.args[0], handle)
                && is_path(&call.args[1], id)
        });
        let ticker = exec.0.iter().any(|(path, _, _, call)| {
            last_segment(path) == "spawn_dictation_partial_ticker"
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
        "68177ec303ffd7ab",
    ),
    (
        "the ticker spawner, dictation-shadow build (`spawn_dictation_partial_ticker`, cfg(feature))",
        "1cec9d24dd8c21fe",
    ),
    ("the VAD spawner (`spawn_vad_stream`)", "8e8a87889467da65"),
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

/// Starts the text of a pinned item that was not found exactly once.
const MULTIPLICITY: &str = "MULTIPLICITY: ";

/// The texts of the five pinned items of `source`, in [`PINS`] order. A target found zero or
/// several times is a `MULTIPLICITY` text, which fails its pin by name.
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
        match ready_callback(&file, entry, receiver) {
            Ok(closure) => text_between(source, closure.span().start(), closure.span().end()),
            Err(defect) => format!("{MULTIPLICITY}{defect}"),
        }
    };
    // A missing item is its own text, so the pin fails on it instead of the test panicking.
    let one = |found: Vec<String>, name: &str| -> String {
        match found.as_slice() {
            [only] => only.clone(),
            other => format!(
                "{MULTIPLICITY}expected exactly one {name}, found {}",
                other.len()
            ),
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
    let texts = pinned_texts(source);
    let found: Vec<String> = texts
        .iter()
        .filter(|text| !text.starts_with(MULTIPLICITY))
        .cloned()
        .collect();
    let mut digests = normalised_digests(&found).into_iter();
    PINS.iter()
        .zip(&texts)
        .filter_map(|((name, pinned), text)| {
            if let Some(why) = text.strip_prefix(MULTIPLICITY) {
                return Some(format!(
                    "S8 pin: {name} is not found exactly once ({why}). Every pinned target must \
                     have exactly one match; pin each supported variant explicitly, with a \
                     reviewed justification that the ticker and the VAD still start for every \
                     eligible recording (D-S2c-16 amendment 1)."
                ));
            }
            let digest = digests.next().expect("one digest for each found text");
            (digest != *pinned).then(|| {
                format!(
                    "S8 pin: {name} changed (digest {digest}, pinned {pinned}). Re-pin only with \
                     a reviewed justification that the ticker and the VAD still start for every \
                     eligible recording (D-S2c-16 amendment 1)."
                )
            })
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

/// Sol's round-3 plant: R104.2 split into two `cfg` branches. The original callback stays
/// first in source order under `dictation-shadow`, and VAD startup is dropped from the
/// default branch. Resolving a callback to the first match kept every digest and the
/// structural check green; two calls to the receiver must fail closed.
const SPLIT_R104_2: Control = (
    "R104.2 split into two cfg branches, VAD dropped from the default branch",
    &[(
        "    publish_recording_ready_with(\n        app_handle,\n        state,\n        recording_id,\n        context_ready_at,\n        ready_op,\n        audio_lifecycle::is_dictation_recording,\n        |app_handle, recording_id, register| {\n            spawn_vad_stream(app_handle.clone(), recording_id);\n            spawn_dictation_partial_ticker(app_handle.clone(), recording_id, register);\n        },\n    )\n}",
        "    #[cfg(feature = \"dictation-shadow\")]\n    {\n        return publish_recording_ready_with(\n            app_handle,\n            state,\n            recording_id,\n            context_ready_at,\n            ready_op,\n            audio_lifecycle::is_dictation_recording,\n            |app_handle, recording_id, register| {\n                spawn_vad_stream(app_handle.clone(), recording_id);\n                spawn_dictation_partial_ticker(app_handle.clone(), recording_id, register);\n            },\n        );\n    }\n    #[cfg(not(feature = \"dictation-shadow\"))]\n    {\n        publish_recording_ready_with(\n            app_handle,\n            state,\n            recording_id,\n            context_ready_at,\n            ready_op,\n            audio_lifecycle::is_dictation_recording,\n            |app_handle, recording_id, register| {\n                spawn_dictation_partial_ticker(app_handle.clone(), recording_id, register);\n            },\n        )\n    }\n}",
    )],
    &["R104.2 ready callback"],
);

/// Sol's round-4 plant, in two spellings of the default branch's receiver: the bridge's
/// `Ready` call split by `cfg`, the original callback kept first under `dictation-shadow`, and
/// a path-qualified call in the default branch whose callback omits VAD startup. A lookup that
/// counts only the bare receiver name saw one call and passed every digest.
macro_rules! split_bridge {
    ($name:literal, $receiver:literal) => {
        (
            $name,
            &[
                (
                    "    event: AudioLifecycleEvent,\n) {\n    handle_audio_lifecycle_events(",
                    "    event: AudioLifecycleEvent,\n) {\n    #[cfg(feature = \"dictation-shadow\")]\n    {\n    handle_audio_lifecycle_events(",
                ),
                (
                    "            spawn_dictation_partial_ticker(app_handle, recording_id, register);\n        },\n    );\n}\n\n/// [`handle_audio_lifecycle`] with its spawns",
                    concat!(
                        "            spawn_dictation_partial_ticker(app_handle, recording_id, register);\n        },\n    );\n    }\n    #[cfg(not(feature = \"dictation-shadow\"))]\n    {\n        ",
                        $receiver,
                        "(\n            app_handle,\n            recording_id,\n            event,\n            auto_stop_dispatch(\n                |app_handle, state| TauriHost::new(app_handle.clone(), state.performance.clone()),\n                |recording_id, _owner, stop| {\n                    spawn_auto_stop(recording_id, stop);\n                },\n            ),\n            |app_handle, recording_id, register| {\n                spawn_dictation_partial_ticker(app_handle, recording_id, register);\n            },\n        );\n    }\n}\n\n/// [`handle_audio_lifecycle`] with its spawns"
                    ),
                ),
            ] as &[(&str, &str)],
            &["the bridge `Ready` ready callback"] as &[&str],
        )
    };
}

const SPLIT_BRIDGE_SELF: Control = split_bridge!(
    "bridge `Ready` split by cfg, `self::`-qualified default branch without VAD",
    "self::handle_audio_lifecycle_events"
);
const SPLIT_BRIDGE_CRATE: Control = split_bridge!(
    "bridge `Ready` split by cfg, `crate::`-qualified default branch without VAD",
    "crate::commands::recording::handle_audio_lifecycle_events"
);

/// A second `spawn_vad_stream`: a pinned function found twice must fail, not resolve to one.
const DUPLICATE_VAD: Control = (
    "a second `spawn_vad_stream`",
    &[(
        "fn spawn_vad_stream<R: tauri::Runtime>(app: tauri::AppHandle<R>, recording_id: u64) {",
        "#[cfg(test)]\nfn spawn_vad_stream(_other: u8) {}\n\nfn spawn_vad_stream<R: tauri::Runtime>(app: tauri::AppHandle<R>, recording_id: u64) {",
    )],
    &["the VAD spawner"],
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
    SPLIT_R104_2,
    SPLIT_BRIDGE_SELF,
    SPLIT_BRIDGE_CRATE,
    DUPLICATE_VAD,
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

/// The two multiplicity plants fail the structural check as well as their pin: the checker
/// reads exactly one receiver and exactly one function, and says which count was wrong.
#[test]
fn a_second_match_fails_the_structural_check_and_names_the_count() {
    let source = production_source();
    for ((name, replacements, _), expected) in [
        (
            SPLIT_R104_2,
            "expected exactly one call to `publish_recording_ready_with`",
        ),
        (
            SPLIT_BRIDGE_SELF,
            "expected exactly one call to `handle_audio_lifecycle_events`",
        ),
        (
            SPLIT_BRIDGE_CRATE,
            "expected exactly one call to `handle_audio_lifecycle_events`",
        ),
        (DUPLICATE_VAD, "expected one `spawn_vad_stream`, found 2"),
    ] {
        let planted = replacements
            .iter()
            .fold(source.clone(), |text, (from, to)| mutated(&text, from, to));
        let defects = handoff_defects(&planted);
        assert!(
            defects.iter().any(|defect| defect.contains(expected)),
            "plant `{name}` was not caught by the structural check as `{expected}`: {defects:#?}"
        );
        let pins = pin_defects(&planted);
        assert!(
            pins.iter()
                .any(|pin| pin.contains("not found exactly once")),
            "plant `{name}`'s pin must say the target is not found exactly once: {pins:#?}"
        );
    }
}
