//! Production-flow regressions for the microphone feedback. They drive the same
//! orchestration `Backend` runs (`tick`, `prepare_check`, `run_check`,
//! `cancel_check`, the report actions and the projections) over a recording
//! service double, including the failure and race paths. Every payload is
//! synthetic and shaped like the service's wire format; nothing touches a
//! device, the pasteboard or private content.
use super::report::summary;
use super::*;
use crate::settings::schema::Page;
use futures::executor::block_on;
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

type Hook = Arc<dyn Fn(&Preview) + Send + Sync>;
type Answers = HashMap<String, VecDeque<Result<Value, String>>>;
type Responder = Arc<dyn Fn(&Value) -> Result<Value, String> + Send + Sync>;

const RUN: &str = "run_microphone_startup_benchmark";
const CANCEL: &str = "cancel_microphone_startup_benchmark";
const SAVE: &str = "save_microphone_startup_benchmark_report";
const PERMISSION: &str = "check_microphone_permission_status";
const STATUS: &str = "get_microphone_preview_status";

#[derive(Default)]
struct Fake {
    preview: Arc<Preview>,
    settings: Mutex<Value>,
    settings_hook: Mutex<Option<Hook>>,
    answers: Mutex<Answers>,
    responders: Mutex<HashMap<String, Responder>>,
    claim: Mutex<HashMap<String, Hook>>,
    during: Mutex<HashMap<String, Hook>>,
    log: Mutex<Vec<(String, Value)>>,
    clipboard: Mutex<Vec<String>>,
    clipboard_fails: AtomicBool,
    changes: AtomicU64,
}
impl Fake {
    fn new() -> Self {
        Self::with(Preview::default())
    }
    fn with(preview: Preview) -> Self {
        Self {
            preview: Arc::new(preview),
            settings: Mutex::new(base()),
            ..Default::default()
        }
    }
    fn answer(&self, command: &str, result: Result<Value, String>) {
        lock(&self.answers)
            .entry(command.into())
            .or_default()
            .push_back(result);
    }
    fn respond(
        &self,
        command: &str,
        f: impl Fn(&Value) -> Result<Value, String> + Send + Sync + 'static,
    ) {
        lock(&self.responders).insert(command.into(), Arc::new(f));
    }
    /// Runs when the service has the request but has not admitted it yet.
    fn on_claim(&self, command: &str, f: impl Fn(&Preview) + Send + Sync + 'static) {
        lock(&self.claim).insert(command.into(), Arc::new(f));
    }
    /// Runs while the service is executing the request.
    fn meanwhile(&self, command: &str, f: impl Fn(&Preview) + Send + Sync + 'static) {
        lock(&self.during).insert(command.into(), Arc::new(f));
    }
    fn on_settings(&self, f: impl Fn(&Preview) + Send + Sync + 'static) {
        *lock(&self.settings_hook) = Some(Arc::new(f));
    }
    fn calls(&self, command: &str) -> Vec<Value> {
        lock(&self.log)
            .iter()
            .filter(|(name, _)| name == command)
            .map(|(_, args)| args.clone())
            .collect()
    }
    fn changes(&self) -> u64 {
        self.changes.load(Ordering::SeqCst)
    }
    fn run(&self, command: &str, args: Value) -> Result<Value, String> {
        lock(&self.log).push((command.into(), args.clone()));
        let hook = lock(&self.during).get(command).cloned();
        if let Some(hook) = hook {
            hook(&self.preview);
        }
        let responder = lock(&self.responders).get(command).cloned();
        if let Some(responder) = responder {
            return responder(&args);
        }
        lock(&self.answers)
            .get_mut(command)
            .and_then(VecDeque::pop_front)
            .unwrap_or(Ok(Value::Null))
    }
}
impl Port for Fake {
    fn preview(&self) -> &Arc<Preview> {
        &self.preview
    }
    fn settings(&self) -> impl Future<Output = Result<Value, String>> + Send {
        let hook = lock(&self.settings_hook).clone();
        if let Some(hook) = hook {
            hook(&self.preview);
        }
        let settings = Ok(lock(&self.settings).clone());
        async move { settings }
    }
    fn call(
        &self,
        command: &str,
        args: Value,
    ) -> impl Future<Output = Result<Value, String>> + Send {
        let result = self.run(command, args);
        async move { result }
    }
    fn effect(
        &self,
        command: &str,
        args: Value,
        live: Live,
    ) -> impl Future<Output = Result<Value, String>> + Send {
        // `Services::request_guarded` refuses a revoked lease up front, and
        // the service asks again once it has claimed the work.
        let result = if !live() {
            Err(PRESENTATION.to_string())
        } else {
            let claim = lock(&self.claim).get(command).cloned();
            if let Some(claim) = claim {
                claim(&self.preview);
            }
            if live() {
                self.run(command, args)
            } else {
                Err(PRESENTATION.to_string())
            }
        };
        async move { result }
    }
    fn copy_text(&self, text: &str) -> Result<(), String> {
        if self.clipboard_fails.load(Ordering::SeqCst) {
            return Err("Clipboard unavailable".into());
        }
        lock(&self.clipboard).push(text.into());
        Ok(())
    }
    fn changed(&self) {
        self.changes.fetch_add(1, Ordering::SeqCst);
    }
}

fn base() -> Value {
    json!({
        "microphone":"synthetic.saved",
        "vadSensitivity":50,
        "smartAutoMicrophoneEnabled":false,
        "smartAutoApprovedDeviceIds":["synthetic.a","synthetic.b"],
        "smartAutoPreferredDeviceIds":["synthetic.a"],
        "smartAutoAllowContinuity":false,
        "smartAutoProbeEnabled":true,
        "benchmarkAutoSave":false,
        "benchmarkOutputDir":"",
    })
}
fn idle() -> Value {
    json!({"previewId":null,"state":"idle"})
}
fn running(id: u64, state: &str) -> Value {
    json!({"previewId":id,"state":state,"stillConnecting":false,"errorKind":null,"message":null})
}
fn shown(preview: &Preview) -> Startup {
    startup(
        &lock(&preview.check),
        &lock(&preview.reports),
        &lock(&preview.marks),
    )
}
fn temp() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("murmur-mic-report-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
/// A Recording-page visit and the lease its preview loop would hold.
fn recording(fake: &Fake) -> Live {
    let epoch = fake.preview.visit(Some(Page::Recording));
    let preview = fake.preview.clone();
    Arc::new(move || preview.epoch.load(Ordering::SeqCst) == epoch)
}

fn attempt(
    pass: u8,
    backend: &str,
    outcome: &str,
    pcm: Option<f64>,
    active: Option<f64>,
    failure: Option<(&str, &str)>,
    step: Option<(&str, &str)>,
) -> Value {
    json!({
        "resolutionPass":pass,"backend":backend,"outcome":outcome,
        "attemptStartToFirstPcmMs":pcm,"activeElapsedMs":active,"attemptBudgetMs":5000.0,
        "failureKind":failure.map(|f| f.0),"failurePhase":failure.map(|f| f.1),
        "lastSetupStep":step.map(|s| s.0),"lastSetupTransition":step.map(|s| s.1),
    })
}
/// Cycle 1 on AUHAL; cycle 2 falls back from a failed AUHAL attempt to CPAL;
/// cycle 3 fails outright.
fn report() -> Value {
    let order = json!(["auhal", "cpal"]);
    json!({
        "schemaVersion":1,"runId":"run-1","startedAt":"2026-01-02T03:04:05.000Z",
        "completedCycles":3,"requestedCycles":5,"cancelled":true,"appVersion":"1.2.3",
        "cycles":[
            {"cycle":1,"outcome":"ready","cycleStartToFirstPcmMs":100.0,"backend":"auhal","backendOrder":order,"backendOrderSource":"default","fallbackOccurred":false,"failureKind":null,"lastSetupStep":null,"lastSetupTransition":null,
             "attempts":[attempt(1,"auhal","ready",Some(90.0),Some(88.0),None,None)]},
            {"cycle":2,"outcome":"ready","cycleStartToFirstPcmMs":200.0,"backend":"cpal","backendOrder":order,"backendOrderSource":"default","fallbackOccurred":true,"failureKind":null,"lastSetupStep":"stream_start","lastSetupTransition":"completed",
             "attempts":[
                attempt(1,"auhal","failed",None,None,Some(("device_unavailable","stream_build")),Some(("set_current_device","entered"))),
                attempt(1,"cpal","ready",Some(140.0),Some(139.0),None,Some(("stream_start","completed")))]},
            {"cycle":3,"outcome":"failed","cycleStartToFirstPcmMs":null,"backend":null,"backendOrder":order,"backendOrderSource":"default","fallbackOccurred":false,"failureKind":"device_busy","lastSetupStep":null,"lastSetupTransition":null,"attempts":[]}
        ]
    })
}
fn full_report(run_id: &str, started: &str) -> Value {
    let mut full = report();
    full["runId"] = json!(run_id);
    full["startedAt"] = json!(started);
    full["completedCycles"] = json!(5);
    full["cancelled"] = json!(false);
    full
}
fn finishes_with(fake: &Fake, started: &'static str) {
    fake.respond(RUN, move |args| {
        Ok(full_report(
            args["request"]["runId"].as_str().unwrap_or_default(),
            started,
        ))
    });
}
/// One complete run through the production start and run path.
fn complete(fake: &Fake, started: &'static str) {
    finishes_with(fake, started);
    let run = block_on(prepare_check(fake)).unwrap().unwrap();
    block_on(run_check(fake, run));
}

#[test]
fn startup_summary_projects_attempts_order_budgets_failures_and_backend_stats() {
    let lines = summary(&report());
    let expected = [
        "Startup results · 3/5 cycles · cancelled early · macOS · Murmur v1.2.3",
        "Median 150 ms · P95 200 ms · Ready 2/3 · Fallbacks 1",
        "AUHAL won 1; CPAL won 1. Range 100 ms–200 ms. Diagnostic cycles read the immutable production backend order and never retrain its first-PCM preference.",
        "AUHAL attempts: 1 ready · 1 failed · 2 total · Median 90 ms · P95 90 ms · Max 90 ms",
        "CPAL attempts: 1 ready · 0 failed · 1 total · Median 140 ms · P95 140 ms · Max 140 ms",
        "Cycle 1: Ready · AUHAL · AUHAL → CPAL · default order · 100 ms · No native setup step reported",
        "Attempt P1 AUHAL · ready in 90 ms · 88 ms active · 5000 ms budget",
        "Cycle 2: Ready · fallback · CPAL · AUHAL → CPAL · default order · 200 ms · Stream start completed",
        "Attempt P1 AUHAL · failed before elapsed timing was confirmed · 5000 ms budget · Device unavailable (Stream build) · Set current device entered",
        "Attempt P1 CPAL · ready in 140 ms · 139 ms active · 5000 ms budget · Stream start completed",
        "Cycle 3: Failed · — · AUHAL → CPAL · default order · — · Device busy · No native setup step reported",
    ];
    assert_eq!(lines, expected);
}

#[test]
fn startup_summary_names_the_real_backends_and_stays_bounded() {
    // The service serializes the backend enum: `auhal` or `cpal`.
    assert_eq!(report::backend(&json!("auhal")), "AUHAL");
    assert_eq!(report::backend(&json!("cpal")), "CPAL");
    assert_eq!(report::backend(&json!("something_else")), "Unknown backend");
    assert_eq!(report::backend(&Value::Null), "—");
    let empty = summary(
        &json!({"runId":"r","completedCycles":0,"requestedCycles":5,"cancelled":true,"appVersion":"1","cycles":[]}),
    );
    assert!(empty.contains(&"The run was cancelled before its first cycle completed.".to_string()));
    assert_eq!(empty[1], "Median — · P95 — · Ready 0/0 · Fallbacks 0");
    let attempts: Vec<Value> = (0..9)
        .map(|_| attempt(1, "cpal", "ready", Some(1.0), Some(1.0), None, None))
        .collect();
    let cycles: Vec<Value> = (0..99)
        .map(|n| json!({"cycle":n,"outcome":"ready","cycleStartToFirstPcmMs":1.0,"backend":"cpal","attempts":attempts}))
        .collect();
    let many = json!({"runId":"r","completedCycles":99,"requestedCycles":99,"cancelled":false,"appVersion":"1","cycles":cycles});
    let lines = summary(&many);
    assert!(lines.len() <= 192 && lines.iter().all(|l| l.chars().count() <= 512));
}

#[test]
fn startup_progress_uses_shipped_wording_and_the_real_backend_names() {
    assert_eq!(
        progress(None),
        ("Preparing production capture path…".into(), 0)
    );
    let running = json!({"runId":"run-1","completedCycles":1,"totalCycles":5,"currentCycle":2,"phase":"capturing","backend":"cpal","fallbackOccurred":true,"lastSetupStep":"stream_start","lastSetupTransition":"started"});
    assert_eq!(
        progress(Some(&running)),
        (
            "Cycle 2 of 5 · CPAL · fallback · Stream start started".into(),
            20
        )
    );
    let waiting = json!({"completedCycles":0,"totalCycles":5,"currentCycle":0,"phase":"waiting"});
    assert_eq!(
        progress(Some(&waiting)).0,
        "Waiting for exclusive microphone access…"
    );
    let done = json!({"completedCycles":5,"totalCycles":5,"currentCycle":5,"phase":"complete"});
    assert_eq!(progress(Some(&done)), ("Finishing report…".into(), 100));
}

#[test]
fn startup_outcome_is_kept_after_the_run_and_rejects_mismatched_reports() {
    let preview = Preview::default();
    *lock(&preview.check) = Check {
        run: Some("run-1".into()),
        progress: Some(json!({"currentCycle":1})),
        cancelling: true,
        ..Default::default()
    };
    // Another run's report is not this run's result.
    assert!(finish_check(&preview.check, "run-1", Ok(json!({"runId":"other"})), true).is_none());
    let s = shown(&preview);
    assert_eq!(
        s.error.as_deref(),
        Some("Murmur returned an invalid microphone startup report.")
    );
    assert!(s.progress.is_none() && !s.cancelling && s.summary.is_empty());

    lock(&preview.check).run = Some("run-2".into());
    assert!(shown(&preview).progress.is_some());
    // A finished older run cannot overwrite the current one.
    assert!(finish_check(&preview.check, "run-1", Err("stale".into()), true).is_none());
    assert_eq!(lock(&preview.check).run.as_deref(), Some("run-2"));
    finish_check(
        &preview.check,
        "run-2",
        Err("The microphone is busy.".into()),
        true,
    );
    let s = shown(&preview);
    assert_eq!(s.error.as_deref(), Some("The microphone is busy."));
    assert!(s.progress.is_none());

    lock(&preview.check).run = Some("run-1".into());
    let kept = finish_check(&preview.check, "run-1", Ok(report()), true);
    assert_eq!(kept, Some(report()));
    assert!(shown(&preview).error.is_none());
}

#[test]
fn startup_progress_is_recorded_only_for_the_running_run() {
    let preview = Preview::default();
    let event = |run: &str| json!({"runId":run,"completedCycles":1,"totalCycles":5,"currentCycle":2,"phase":"capturing"});
    preview.observe("microphone-startup-benchmark-progress", &event("run-1"));
    assert!(lock(&preview.check).progress.is_none());
    lock(&preview.check).run = Some("run-1".into());
    preview.observe("microphone-startup-benchmark-progress", &event("stale"));
    assert!(lock(&preview.check).progress.is_none());
    preview.observe("microphone-startup-benchmark-progress", &event("run-1"));
    assert_eq!(shown(&preview).progress.unwrap().1, 20);
}

// Row 1: leaving the page can never leave a capture running behind it.

#[test]
fn leaving_performance_while_the_run_reads_settings_never_starts_capture() {
    let fake = Fake::new();
    fake.preview.visit(Some(Page::Performance));
    // The page change lands while the settings read is still in flight.
    fake.on_settings(|preview| {
        preview.visit(Some(Page::General));
        assert!(preview.leave().is_some());
    });
    assert!(block_on(prepare_check(&fake)).unwrap().is_none());
    assert!(fake.calls(RUN).is_empty());
    let s = shown(&fake.preview);
    assert!(s.progress.is_none() && s.error.is_none() && s.summary.is_empty());
}

#[test]
fn leaving_before_the_service_is_asked_refuses_the_run() {
    let fake = Fake::new();
    fake.preview.visit(Some(Page::Performance));
    let run = block_on(prepare_check(&fake)).unwrap().unwrap();
    fake.preview.visit(Some(Page::Recording));
    assert!(fake.preview.leave().is_some());
    block_on(run_check(&fake, run));
    assert!(fake.calls(RUN).is_empty());
    let s = shown(&fake.preview);
    assert!(s.progress.is_none() && s.error.is_none());
}

#[test]
fn leaving_between_the_service_claim_and_admission_refuses_the_run() {
    let fake = Fake::new();
    fake.preview.visit(Some(Page::Performance));
    finishes_with(&fake, "2026-01-02T03:04:05.000Z");
    fake.on_claim(RUN, |preview| {
        preview.visit(Some(Page::General));
        preview.leave();
    });
    let run = block_on(prepare_check(&fake)).unwrap().unwrap();
    block_on(run_check(&fake, run));
    // The service refused it after claiming: no cycle ran, nothing is shown.
    assert!(fake.calls(RUN).is_empty());
    let s = shown(&fake.preview);
    assert!(s.progress.is_none() && s.error.is_none() && s.summary.is_empty());
    assert!(lock(&fake.preview.check).run.is_none());
}

#[test]
fn cancel_before_the_service_claims_the_run_is_effective() {
    let fake = Fake::new();
    fake.preview.visit(Some(Page::Performance));
    let run = block_on(prepare_check(&fake)).unwrap().unwrap();
    // The service answers "not running" for a run it has not claimed.
    fake.answer(CANCEL, Ok(json!(false)));
    block_on(cancel_check(&fake));
    let s = shown(&fake.preview);
    assert!(s.cancelling && s.error.is_none());
    block_on(run_check(&fake, run));
    assert!(fake.calls(RUN).is_empty());
    assert_eq!(fake.calls(CANCEL).len(), 1);
    let s = shown(&fake.preview);
    assert!(s.progress.is_none() && !s.cancelling && s.error.is_none() && s.summary.is_empty());
}

#[test]
fn a_run_that_finishes_after_its_page_was_left_is_discarded() {
    let fake = Fake::new();
    *lock(&fake.settings) = {
        let mut settings = base();
        settings["benchmarkAutoSave"] = json!(true);
        settings
    };
    fake.preview.visit(Some(Page::Performance));
    finishes_with(&fake, "2026-01-02T03:04:05.000Z");
    fake.meanwhile(RUN, |preview| {
        preview.visit(Some(Page::General));
        preview.leave();
    });
    let run = block_on(prepare_check(&fake)).unwrap().unwrap();
    block_on(run_check(&fake, run));
    assert_eq!(fake.calls(RUN).len(), 1);
    assert!(fake.calls(SAVE).is_empty());
    let s = shown(&fake.preview);
    assert!(s.progress.is_none() && s.error.is_none() && s.runs.is_empty());
}

#[test]
fn staying_on_performance_keeps_the_run_and_other_pages_cannot_start_one() {
    let fake = Fake::new();
    fake.preview.visit(Some(Page::General));
    assert_eq!(
        block_on(prepare_check(&fake)).err().as_deref(),
        Some("Open Performance settings to test microphone startup")
    );
    assert!(lock(&fake.preview.check).run.is_none());
    fake.preview.visit(Some(Page::Performance));
    let _run = block_on(prepare_check(&fake)).unwrap().unwrap();
    // Opening the same page again is not leaving it.
    fake.preview.visit(Some(Page::Performance));
    assert!(fake.preview.leave().is_none());
    assert_eq!(
        block_on(prepare_check(&fake)).err().as_deref(),
        Some("A microphone startup benchmark is already running")
    );
}

#[test]
fn partial_runs_are_shown_for_this_launch_but_never_persisted_or_auto_saved() {
    let dir = temp();
    let fake = Fake::with(Preview::persisted(&dir));
    *lock(&fake.settings) = {
        let mut settings = base();
        settings["benchmarkAutoSave"] = json!(true);
        settings["benchmarkOutputDir"] = json!("/synthetic/out");
        settings
    };
    fake.preview.visit(Some(Page::Performance));
    fake.respond(RUN, |args| {
        let mut partial = report();
        partial["runId"] = args["request"]["runId"].clone();
        Ok(partial)
    });
    // Cancel arrives while the service is running the cycles.
    fake.meanwhile(RUN, |preview| {
        if let Some(abort) = &lock(&preview.check).abort {
            abort.store(true, Ordering::SeqCst);
        }
    });
    let run = block_on(prepare_check(&fake)).unwrap().unwrap();
    block_on(run_check(&fake, run));
    let s = shown(&fake.preview);
    assert!(s.summary[0].contains("cancelled early"));
    assert!(s.error.is_none() && s.progress.is_none());
    assert!(fake.calls(SAVE).is_empty());
    assert!(!dir.join("microphone-startup-reports.json").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn full_runs_are_kept_selectable_persisted_and_cleared() {
    let dir = temp();
    let file = dir.join("microphone-startup-reports.json");
    let fake = Fake::with(Preview::persisted(&dir));
    fake.preview.visit(Some(Page::Performance));
    complete(&fake, "2026-01-02T03:04:05.000Z");
    let s = shown(&fake.preview);
    assert_eq!(s.runs, vec!["2026-01-02 03:04:05 UTC · 5/5 cycles"]);
    assert_eq!(
        s.summary[0],
        summary(&lock(&fake.preview.reports).current().cloned().unwrap())[0]
    );
    assert!(file.exists());
    // A newer run goes first and is selected; the older one stays reachable.
    complete(&fake, "2026-01-03T00:00:00.000Z");
    let s = shown(&fake.preview);
    assert_eq!(s.runs.len(), 2);
    assert!(s.runs[0].starts_with("2026-01-03") && s.selected == 0);
    lock(&fake.preview.reports).select(1);
    assert_eq!(shown(&fake.preview).selected, 1);
    // The history survives a relaunch.
    let relaunched = Preview::persisted(&dir);
    assert_eq!(shown(&relaunched).runs.len(), 2);
    // While a run is active the history is not shown over its progress.
    lock(&fake.preview.check).run = Some("active".into());
    let s = shown(&fake.preview);
    assert!(s.runs.is_empty() && s.summary.is_empty() && s.progress.is_some());
    *lock(&fake.preview.check) = Check::default();
    clear_reports(&fake);
    let s = shown(&fake.preview);
    assert!(s.runs.is_empty() && s.summary.is_empty() && s.error.is_none());
    assert!(!file.exists());
    assert!(shown(&Preview::persisted(&dir)).runs.is_empty());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_history_that_cannot_be_written_is_reported_but_the_run_is_kept() {
    let dir = temp();
    // A directory where the file belongs makes the atomic replace fail.
    std::fs::create_dir(dir.join("microphone-startup-reports.json")).unwrap();
    let fake = Fake::with(Preview::persisted(&dir));
    fake.preview.visit(Some(Page::Performance));
    complete(&fake, "2026-01-02T03:04:05.000Z");
    let s = shown(&fake.preview);
    assert_eq!(
        s.error.as_deref(),
        Some("Could not save this run to the history on this Mac.")
    );
    assert_eq!(s.runs.len(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn auto_save_writes_full_runs_through_the_typed_command_and_reports_failure() {
    let fake = Fake::new();
    *lock(&fake.settings) = {
        let mut settings = base();
        settings["benchmarkAutoSave"] = json!(true);
        settings["benchmarkOutputDir"] = json!("/synthetic/out");
        settings
    };
    fake.preview.visit(Some(Page::Performance));
    complete(&fake, "2026-01-02T03:04:05.000Z");
    let saves = fake.calls(SAVE);
    assert_eq!(saves.len(), 1);
    assert_eq!(saves[0]["outputDir"], "/synthetic/out");
    assert_eq!(saves[0]["report"]["startedAt"], "2026-01-02T03:04:05.000Z");
    assert!(shown(&fake.preview).error.is_none());
    fake.respond(SAVE, |_| Err("The folder is not writable".into()));
    complete(&fake, "2026-01-03T00:00:00.000Z");
    assert_eq!(
        shown(&fake.preview).error.as_deref(),
        Some("Run completed, but the report could not be auto-saved: The folder is not writable")
    );
    // The run is still there to read, copy or save by hand.
    assert_eq!(shown(&fake.preview).runs.len(), 2);
}

#[test]
fn copy_json_uses_the_application_clipboard_and_reports_failure() {
    let fake = Fake::new();
    // Nothing to copy yet.
    assert!(copy_report(&fake).is_none());
    assert!(lock(&fake.clipboard).is_empty());
    fake.preview.visit(Some(Page::Performance));
    complete(&fake, "2026-01-02T03:04:05.000Z");
    let token = copy_report(&fake).unwrap();
    let copied: Value = serde_json::from_str(&lock(&fake.clipboard)[0]).unwrap();
    assert_eq!(
        copied,
        lock(&fake.preview.reports).current().cloned().unwrap()
    );
    assert!(shown(&fake.preview).copied);
    // The confirmation ends after its interval, and only for its own token.
    assert!(!lock(&fake.preview.marks).expire(token + 100));
    assert!(shown(&fake.preview).copied);
    assert!(lock(&fake.preview.marks).expire(token));
    assert!(!shown(&fake.preview).copied);
    fake.clipboard_fails.store(true, Ordering::SeqCst);
    assert!(copy_report(&fake).is_none());
    let s = shown(&fake.preview);
    assert_eq!(
        s.error.as_deref(),
        Some("Could not copy report: Clipboard unavailable")
    );
    assert!(!s.copied);
}

#[test]
fn save_to_file_shows_saving_then_saved_and_explains_failure() {
    let fake = Fake::new();
    let live: Live = Arc::new(|| true);
    assert!(block_on(save_report(&fake, live.clone())).is_none());
    assert!(fake.calls(SAVE).is_empty());
    fake.preview.visit(Some(Page::Performance));
    complete(&fake, "2026-01-02T03:04:05.000Z");
    let before = fake.changes();
    let token = block_on(save_report(&fake, live.clone())).unwrap();
    // "Saving…" was announced before the command returned.
    assert!(fake.changes() > before);
    let saves = fake.calls(SAVE);
    assert_eq!(saves.len(), 1);
    assert_eq!(saves[0]["outputDir"], "");
    assert_eq!(saves[0]["report"]["runId"], full_run_id(&fake));
    let s = shown(&fake.preview);
    assert!(s.saved && !s.saving && s.error.is_none());
    assert!(lock(&fake.preview.marks).expire(token));
    let s = shown(&fake.preview);
    assert!(!s.saved && !s.saving);
    fake.respond(SAVE, |_| Err("The folder is not writable".into()));
    assert!(block_on(save_report(&fake, live)).is_none());
    let s = shown(&fake.preview);
    assert!(!s.saved && !s.saving);
    assert_eq!(
        s.error.as_deref(),
        Some("Could not save report: The folder is not writable")
    );
}
fn full_run_id(fake: &Fake) -> Value {
    lock(&fake.preview.reports).current().unwrap()["runId"].clone()
}

// Row 2: a stop in flight cannot take a newer visit's preview ownership.

#[test]
fn a_stop_in_flight_when_the_page_is_left_leaves_the_newer_previews_ownership_alone() {
    for outcome in [Ok(Value::Null), Err("The microphone stopped".to_string())] {
        let fake = Fake::new();
        let live = recording(&fake);
        fake.preview.id.store(3, Ordering::SeqCst);
        fake.answer(PERMISSION, Ok(json!("granted")));
        fake.answer(STATUS, Ok(running(3, "active")));
        fake.answer("stop_microphone_preview", outcome);
        // Leave and return while the stop is awaited: the newer visit opens
        // its own preview, which the Settings meter must keep following.
        fake.meanwhile("stop_microphone_preview", |preview| {
            preview.visit(Some(Page::Recording));
            preview.id.store(9, Ordering::SeqCst);
        });
        block_on(tick(&fake, &live, &mut Loop::default()));
        assert_eq!(fake.preview.id.load(Ordering::SeqCst), 9);
        assert!(lock(&fake.preview.failure).is_none());
        assert_eq!(fake.changes(), 0);
    }
}

#[test]
fn a_stop_on_the_current_visit_releases_its_own_id_or_reports_the_refusal() {
    let fake = Fake::new();
    let live = recording(&fake);
    fake.preview.id.store(3, Ordering::SeqCst);
    fake.answer(PERMISSION, Ok(json!("granted")));
    fake.answer(STATUS, Ok(running(3, "active")));
    block_on(tick(&fake, &live, &mut Loop::default()));
    assert_eq!(fake.preview.id.load(Ordering::SeqCst), 0);
    fake.answer(PERMISSION, Ok(json!("granted")));
    fake.answer(STATUS, Ok(running(4, "active")));
    fake.answer("stop_microphone_preview", Err("Stop refused".into()));
    block_on(tick(&fake, &live, &mut Loop::default()));
    assert_eq!(lock(&fake.preview.failure).as_deref(), Some("Stop refused"));
    assert_eq!(fake.changes(), 1);
}

// Row 3: denied or unconfirmed microphone access is explained, not silent.

#[test]
fn denied_or_unconfirmed_microphone_access_is_explained_with_recovery_guidance() {
    let fake = Fake::new();
    let live = recording(&fake);
    let mut state = Loop::default();
    fake.answer(PERMISSION, Ok(json!("denied")));
    block_on(tick(&fake, &live, &mut state));
    let message = lock(&fake.preview.failure).clone().unwrap();
    assert!(message.contains("denied or revoked"));
    assert!(message.contains("Privacy & Security → Microphone"));
    let test = mic_test(&idle(), Some(message.as_str()), None, 50);
    assert_eq!(test.error.as_deref(), Some(message.as_str()));
    assert!(test.retry);
    assert!(fake.calls(STATUS).is_empty() && fake.calls("start_microphone_preview").is_empty());
    assert_eq!(fake.changes(), 1);
    // The same explanation is not re-announced every tick.
    fake.answer(PERMISSION, Ok(json!("denied")));
    block_on(tick(&fake, &live, &mut state));
    assert_eq!(fake.changes(), 1);
    // A status that cannot be read is not treated as granted.
    fake.answer(PERMISSION, Err("Permission status unavailable".into()));
    block_on(tick(&fake, &live, &mut state));
    let message = lock(&fake.preview.failure).clone().unwrap();
    assert!(message.contains("could not be confirmed"));
    assert_eq!(fake.changes(), 2);
    assert!(fake.calls("start_microphone_preview").is_empty());
    // Granting access clears the explanation and the preview starts.
    fake.answer(PERMISSION, Ok(json!("granted")));
    fake.answer(STATUS, Ok(idle()));
    fake.answer("start_microphone_preview", Ok(json!({"previewId":5})));
    block_on(tick(&fake, &live, &mut state));
    assert!(lock(&fake.preview.failure).is_none());
    assert_eq!(fake.preview.id.load(Ordering::SeqCst), 5);
    assert_eq!(fake.calls("start_microphone_preview").len(), 1);
    assert_eq!(fake.changes(), 3);
}

#[test]
fn undetermined_access_attempts_the_capture_that_asks_macos() {
    let fake = Fake::new();
    let live = recording(&fake);
    fake.answer(PERMISSION, Ok(json!("notDetermined")));
    fake.answer(STATUS, Ok(idle()));
    fake.answer("start_microphone_preview", Ok(json!({"previewId":6})));
    block_on(tick(&fake, &live, &mut Loop::default()));
    assert_eq!(fake.calls("start_microphone_preview").len(), 1);
    assert!(lock(&fake.preview.failure).is_none());
}

// Row 4: Smart Auto Retry follows the shared selector.

#[test]
fn smart_auto_retry_follows_continuity_permission_and_the_lid() {
    use murmur_services::native_services::smart_auto_selection_available as selectable;
    let inventory = |lid: &str| {
        json!({
            "status":"ready","defaultInputId":"built-in","lidState":lid,
            "devices":[
                {"id":"built-in","name":"Built-in","kind":"builtIn","connected":true,"hasInput":true},
                {"id":"iphone","name":"Phone","kind":"continuity","connected":true,"hasInput":true},
            ],
        })
    };
    let ids = |values: &[&str]| values.iter().map(|v| v.to_string()).collect::<Vec<_>>();
    let devices = ["built-in", "iphone"].map(|id| Device {
        id: id.into(),
        label: format!("Synthetic {id}"),
        available: true,
        continuity: id == "iphone",
    });
    let blocked = json!({"state":"blocked","message":"Service wording.","retryAfterMs":null});
    let feedback = |approved: &[&str], continuity: bool, lid: &str| {
        let mut settings = base();
        settings["smartAutoMicrophoneEnabled"] = json!(true);
        settings["smartAutoApprovedDeviceIds"] = json!(approved);
        smart_auto(
            &settings,
            &devices,
            selectable(&ids(approved), &[], continuity, &inventory(lid)),
            Ok(blocked.clone()),
            false,
        )
        .0
    };
    let none = "None of the included microphones are available right now.";
    // A connected, approved iPhone is not usable while Continuity is off.
    assert!(matches!(
        feedback(&["iphone"], false, "open"),
        SmartAuto::Blocked { message, retry: false, .. } if message == none
    ));
    assert!(matches!(
        feedback(&["iphone"], true, "open"),
        SmartAuto::Blocked { message, retry: true, .. } if message == "Service wording."
    ));
    // The built-in microphone is not usable with the lid closed.
    assert!(matches!(
        feedback(&["built-in"], false, "closed"),
        SmartAuto::Blocked { message, retry: false, .. } if message == none
    ));
    assert!(matches!(
        feedback(&["built-in"], false, "open"),
        SmartAuto::Blocked { retry: true, .. }
    ));
}

// A refused sensitivity update stays explained until a decision at the
// requested sensitivity says otherwise.

#[test]
fn a_refused_sensitivity_update_is_not_cleared_by_a_stale_sensitivity_event() {
    let fake = Fake::new();
    let live = recording(&fake);
    lock(&fake.settings)["vadSensitivity"] = json!(80);
    let mut state = Loop::default();
    state.started(7, config(&base()), 50);
    fake.preview.id.store(7, Ordering::SeqCst);
    fake.answer(PERMISSION, Ok(json!("granted")));
    fake.answer(STATUS, Ok(running(7, "active")));
    fake.answer(
        "update_microphone_preview_vad_sensitivity",
        Err("Unavailable".into()),
    );
    block_on(tick(&fake, &live, &mut state));
    assert_eq!(*lock(&fake.preview.vad), Some((7, 80, true)));
    let explained =
        |preview: &Arc<Preview>| mic_test(&running(7, "active"), None, *lock(&preview.vad), 80);
    assert!(explained(&fake.preview).vad_unavailable);
    // The service's next decision still reports the old sensitivity.
    let stale = json!({"previewId":7,"sensitivity":50,"decision":"speech_detected"});
    assert!(!fake.preview.observe("microphone-preview-vad", &stale));
    assert_eq!(*lock(&fake.preview.vad), Some((7, 80, true)));
    assert!(explained(&fake.preview).vad_unavailable);
    // A decision at the requested sensitivity is authoritative again.
    let current = json!({"previewId":7,"sensitivity":80,"decision":"speech_detected"});
    assert!(fake.preview.observe("microphone-preview-vad", &current));
    assert!(!explained(&fake.preview).vad_unavailable);
}
