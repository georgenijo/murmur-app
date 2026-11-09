//! A revocable Settings-page meter. Capture admission and PCM disposal belong
//! to the existing preview owner; no monitor is started by a subscription.
use super::backend::Backend;
use super::microphone_report::{self as report, Marks, Reports, Save};
use super::os::Live;
use crate::settings::model::{Device, MicTest, SmartAuto, Startup};
use serde_json::{Value, json};
use std::future::Future;
use std::sync::{
    Arc, Mutex, MutexGuard,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::time::{Duration, Instant};

/// Synthetic Main event: microphone feedback that no service event announces
/// (local failures, voice-detection decisions, deadlines, benchmark outcomes).
pub const CHANGED: &str = "microphone-test-changed";
/// Contention that resolves by itself; every other refusal suspends the test.
const DICTATION_BUSY: &str = "Finish the current dictation before testing the microphone";
/// What a revoked lease answers, before and after the service admits a request.
const PRESENTATION: &str = "Presentation changed";
const CYCLES: u8 = 5;

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

/// The service and OS operations the microphone feedback needs. `Backend` is
/// the production implementation; tests drive the same logic through a double.
pub trait Port: Send + Sync {
    fn preview(&self) -> &Arc<Preview>;
    fn settings(&self) -> impl Future<Output = Result<Value, String>> + Send;
    fn call(
        &self,
        command: &str,
        args: Value,
    ) -> impl Future<Output = Result<Value, String>> + Send;
    fn effect(
        &self,
        command: &str,
        args: Value,
        live: Live,
    ) -> impl Future<Output = Result<Value, String>> + Send;
    /// The application's clipboard path.
    fn copy_text(&self, text: &str) -> Result<(), String>;
    fn changed(&self);
}
impl Port for Backend {
    fn preview(&self) -> &Arc<Preview> {
        &self.preview
    }
    fn settings(&self) -> impl Future<Output = Result<Value, String>> + Send {
        super::projections::settings(self)
    }
    fn call(
        &self,
        command: &str,
        args: Value,
    ) -> impl Future<Output = Result<Value, String>> + Send {
        Backend::call(self, command, args)
    }
    fn effect(
        &self,
        command: &str,
        args: Value,
        live: Live,
    ) -> impl Future<Output = Result<Value, String>> + Send {
        Backend::effect(self, command, args, live)
    }
    fn copy_text(&self, text: &str) -> Result<(), String> {
        self.services.copy_text(text)
    }
    fn changed(&self) {
        self.events.publish("main", CHANGED, "{}");
    }
}

#[derive(Default)]
pub struct Preview {
    epoch: AtomicU64,
    /// Epoch of the current visit to the Performance page, 0 anywhere else.
    performance: AtomicU64,
    pub id: AtomicU64,
    pub meter: Mutex<Option<(u64, f32, f32)>>,
    /// Latest voice-detection decision: (preview id, sensitivity, unavailable).
    pub vad: Mutex<Option<(u64, u32, bool)>>,
    /// Sensitivity last requested for a preview: (preview id, sensitivity).
    /// Decisions made at another sensitivity are stale and describe nothing.
    vad_target: Mutex<Option<(u64, u32)>>,
    /// A start or stop the service refused. Automatic starts stay suspended
    /// until the configuration changes or the user retries.
    pub failure: Mutex<Option<String>>,
    /// The failure above is the microphone-access explanation.
    permission: AtomicBool,
    pub retry: AtomicBool,
    pub probe_retry_failed: AtomicBool,
    /// When the Smart Auto status last projected stops being current.
    pub smart_deadline: Mutex<Option<Instant>>,
    pub check: Mutex<Check>,
    /// Finished startup runs, newest first.
    pub reports: Mutex<Reports>,
    pub marks: Mutex<Marks>,
}
impl Preview {
    /// A preview whose finished full runs persist under the data directory.
    pub fn persisted(dir: &std::path::Path) -> Self {
        Self {
            reports: Mutex::new(Reports::load(dir)),
            ..Default::default()
        }
    }
    /// Starts a visit to a Settings page (None: Settings closed) and returns its
    /// epoch. Staying on Performance keeps the same visit, so a run survives it.
    pub fn visit(&self, page: Option<crate::settings::schema::Page>) -> u64 {
        use crate::settings::schema::Page;
        let epoch = self.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        if page == Some(Page::Performance) {
            let _ = self
                .performance
                .compare_exchange(0, epoch, Ordering::SeqCst, Ordering::SeqCst);
        } else {
            self.performance.store(0, Ordering::SeqCst);
        }
        self.id.store(0, Ordering::SeqCst);
        epoch
    }
    /// Revokes a run that belongs to a visit that ended, and names it so the
    /// service can be told. The flag is set here, synchronously, so a run that
    /// has not reached the service yet is refused when it asks for admission.
    pub fn leave(&self) -> Option<String> {
        let performance = self.performance.load(Ordering::SeqCst);
        let check = lock(&self.check);
        let run = check.run.clone()?;
        if check.visit == performance {
            return None;
        }
        if let Some(abort) = &check.abort {
            abort.store(true, Ordering::SeqCst);
        }
        Some(run)
    }
    /// Records a Main event that Settings reads back through the projection.
    /// True when the projection must be refreshed for an event the shared
    /// watch list deliberately does not follow.
    pub fn observe(&self, event: &str, value: &Value) -> bool {
        match event {
            "microphone-preview-vad" => {
                let (Some(id), Some(sensitivity)) =
                    (value["previewId"].as_u64(), value["sensitivity"].as_u64())
                else {
                    return false;
                };
                let sensitivity = sensitivity.min(100) as u32;
                // A decision made before the latest request reached the service
                // must not replace what that request reported, as shipped.
                if lock(&self.vad_target)
                    .is_some_and(|(target, wanted)| target == id && wanted != sensitivity)
                {
                    return false;
                }
                let next = Some((id, sensitivity, value["decision"] == "unavailable"));
                let mut vad = lock(&self.vad);
                let changed = *vad != next;
                *vad = next;
                changed
            }
            "microphone-startup-benchmark-progress" => {
                let mut check = lock(&self.check);
                if check.run.is_some() && check.run.as_deref() == value["runId"].as_str() {
                    check.progress = Some(value.clone());
                }
                false
            }
            _ => false,
        }
    }
}
/// One microphone startup diagnostic and its outcome. The finished report
/// lives in `Preview::reports`, so it outlives the request that started it.
#[derive(Default)]
pub struct Check {
    pub run: Option<String>,
    pub progress: Option<Value>,
    pub cancelling: bool,
    pub error: Option<String>,
    /// The Performance visit that started the run and its revocation switch.
    visit: u64,
    abort: Option<Arc<AtomicBool>>,
}

#[cfg(test)]
#[path = "microphone_flow_tests.rs"]
mod flow_tests;
#[cfg(test)]
#[path = "microphone_tests.rs"]
mod tests;

/// What the preview must be opened with for the current settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Config {
    pub device: String,
    pub smart: Option<Value>,
}
pub fn config(settings: &Value) -> Config {
    let smart = (settings["smartAutoMicrophoneEnabled"] == true).then(|| {
        json!({
            "approvedDeviceIds":settings["smartAutoApprovedDeviceIds"],
            "preferredDeviceIds":settings["smartAutoPreferredDeviceIds"],
            "allowContinuity":settings["smartAutoAllowContinuity"],
            "requireRecentSignal":settings["smartAutoProbeEnabled"],
        })
    });
    // Smart Auto resolves the input itself. The service rejects Smart Auto
    // combined with a fixed device (`start_microphone_preview`), so a saved
    // microphone must not travel with it.
    let device = match &smart {
        Some(_) => "system_default",
        None => settings["microphone"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("system_default"),
    };
    Config {
        device: device.into(),
        smart,
    }
}
pub fn vad(settings: &Value) -> u32 {
    settings["vadSensitivity"]
        .as_f64()
        .map_or(0, |v| v.clamp(0., 100.) as u32)
}
pub fn start_args(config: &Config, vad: u32) -> Value {
    json!({"deviceId":config.device,"smartAuto":config.smart,"vadSensitivity":vad})
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Wait,
    Start,
    Stop(u64),
    Vad(u64, u32),
}
/// Reconciles the preview service with the settings. State lives for one visit
/// to the Recording page; a new visit adopts nothing it cannot prove.
#[derive(Default)]
pub struct Loop {
    /// Preview this loop opened with the configuration and sensitivity sent.
    owned: Option<(u64, Config, u32)>,
    pub suspended: Option<Config>,
}
impl Loop {
    pub fn step(&mut self, want: &Config, vad: u32, status: &Value, retry: bool) -> Step {
        if retry || self.suspended.as_ref().is_some_and(|held| held != want) {
            self.suspended = None;
        }
        let state = status["state"].as_str().unwrap_or("idle");
        let Some(id) = status["previewId"].as_u64() else {
            // Idle, or a terminal error that retains its message, not capture.
            if let Some((_, config, _)) = self.owned.take()
                && state == "error"
                && !retry
                && &config == want
            {
                self.suspended = Some(config);
            }
            return if self.suspended.is_some() {
                Step::Wait
            } else {
                Step::Start
            };
        };
        if state == "stopping" {
            return Step::Wait;
        }
        let Some((_, _, sent)) = self
            .owned
            .as_ref()
            .filter(|(owned, config, _)| *owned == id && config == want)
        else {
            // Another configuration, or a capture this visit did not open.
            return if self.suspended.is_some() {
                Step::Wait
            } else {
                Step::Stop(id)
            };
        };
        if state == "error" {
            return Step::Wait;
        }
        if *sent != vad {
            return Step::Vad(id, vad);
        }
        Step::Wait
    }
    pub fn started(&mut self, id: u64, config: Config, vad: u32) {
        self.owned = Some((id, config, vad));
    }
    pub fn stopped(&mut self) {
        self.owned = None;
    }
    pub fn sent(&mut self, vad: u32) {
        if let Some((_, _, sent)) = self.owned.as_mut() {
            *sent = vad;
        }
    }
}

impl Backend {
    pub fn changed(&self) {
        Port::changed(self);
    }
    pub fn preview_page(&self, page: Option<crate::settings::schema::Page>, _attachment: u64) {
        use crate::settings::schema::Page;
        let epoch = self.preview.visit(page);
        self.services.main_hidden();
        if page.is_none() {
            self.navigate(super::backend::Route::Calibration {
                offset: None,
                finish: true,
            });
        }
        // Leaving the diagnostic's page stops its run, as the shipped
        // component does on unmount. The run is revoked before this returns.
        if let Some(run) = self.preview.leave() {
            let b = self.clone();
            self.spawn(async move {
                let _ = b
                    .call("cancel_microphone_startup_benchmark", json!({"runId":run}))
                    .await;
            });
        }
        if page != Some(Page::Recording) {
            return;
        }
        let b = self.clone();
        self.spawn(async move {
            let live: Live = {
                let p = b.preview.clone();
                Arc::new(move || p.epoch.load(Ordering::SeqCst) == epoch)
            };
            let mut state = Loop::default();
            while live() {
                let due = {
                    let mut deadline = lock(&b.preview.smart_deadline);
                    let due = deadline.is_some_and(|at| Instant::now() >= at);
                    if due {
                        deadline.take();
                    }
                    due
                };
                if due {
                    b.changed();
                }
                tick(&b, &live, &mut state).await;
                tokio::time::sleep(Duration::from_millis(400)).await;
            }
        });
    }
}

/// The microphone-access explanation, or None when capture may be attempted.
/// Granted and not-yet-determined proceed: like the shipped pane, opening the
/// input is what asks macOS for access, and the service reports a refusal.
fn permission_block(status: &Result<Value, String>) -> Option<&'static str> {
    use super::dictation_feedback::Notice;
    match status.as_ref().map(Value::as_str) {
        Ok(Some("granted" | "notDetermined")) => None,
        Ok(Some("denied")) => Some(Notice::from_code("microphone_denied").message),
        _ => Some(Notice::from_code("microphone_unknown").message),
    }
}
pub async fn tick<P: Port>(p: &P, live: &Live, state: &mut Loop) {
    let Ok(settings) = p.settings().await else {
        return;
    };
    if !live() {
        return;
    }
    let permission = p
        .call("check_microphone_permission_status", json!({}))
        .await;
    if !live() {
        return;
    }
    let preview = p.preview();
    if let Some(message) = permission_block(&permission) {
        // Say why nothing starts: denied or unconfirmed access is not a
        // silent preview, and the explanation names where to restore it.
        let changed = {
            let mut failure = lock(&preview.failure);
            let changed = failure.as_deref() != Some(message);
            *failure = Some(message.into());
            changed
        };
        preview.permission.store(true, Ordering::SeqCst);
        if changed {
            p.changed();
        }
        return;
    }
    if preview.permission.swap(false, Ordering::SeqCst) && lock(&preview.failure).take().is_some() {
        p.changed();
    }
    let Ok(status) = p.call("get_microphone_preview_status", json!({})).await else {
        return;
    };
    if !live() {
        return;
    }
    let (want, sensitivity) = (config(&settings), vad(&settings));
    let retry = preview.retry.swap(false, Ordering::SeqCst);
    let step = state.step(&want, sensitivity, &status, retry);
    if (retry || state.suspended.is_none()) && lock(&preview.failure).take().is_some() {
        p.changed();
    }
    if status["previewId"].is_null() {
        preview.id.store(0, Ordering::SeqCst);
    }
    match step {
        Step::Wait => {}
        Step::Start => {
            match p
                .effect(
                    "start_microphone_preview",
                    start_args(&want, sensitivity),
                    live.clone(),
                )
                .await
            {
                Ok(started) => {
                    if let Some(id) = started["previewId"].as_u64() {
                        if live() {
                            preview.id.store(id, Ordering::SeqCst);
                            *lock(&preview.vad_target) = Some((id, sensitivity));
                            state.started(id, want, sensitivity);
                        } else {
                            let _ = p
                                .call("cancel_microphone_preview", json!({"previewId":id}))
                                .await;
                        }
                    }
                }
                Err(error) if live() && !error.contains(DICTATION_BUSY) => {
                    state.suspended = Some(want);
                    *lock(&preview.failure) = Some(clip(&error, 512));
                    p.changed();
                }
                Err(_) => {}
            }
        }
        Step::Stop(id) => {
            let stopped = p
                .call("stop_microphone_preview", json!({"previewId":id}))
                .await;
            // A newer visit may have opened its own preview while this stop was
            // in flight. Its id and failure are not this loop's to change.
            if !live() {
                return;
            }
            match stopped {
                Ok(_) => {
                    state.stopped();
                    let _ = preview
                        .id
                        .compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
                }
                Err(error) => {
                    state.suspended = Some(want);
                    *lock(&preview.failure) = Some(clip(&error, 512));
                    p.changed();
                }
            }
        }
        Step::Vad(id, value) => {
            *lock(&preview.vad_target) = Some((id, value));
            let updated = p
                .call(
                    "update_microphone_preview_vad_sensitivity",
                    json!({"previewId":id,"vadSensitivity":value}),
                )
                .await;
            // Reassert nothing on failure: the shipped pane reports it once.
            state.sent(value);
            let mut vad = lock(&preview.vad);
            *vad = match updated {
                Ok(_) => None,
                Err(_) => Some((id, value, true)),
            };
            drop(vad);
            p.changed();
        }
    }
}

/// Live-test feedback for one projection. A local failure wins over the
/// service's own message, then voice detection, as in the shipped pane.
pub fn mic_test(
    status: &Value,
    failure: Option<&str>,
    vad: Option<(u64, u32, bool)>,
    sensitivity: u32,
) -> MicTest {
    let id = status["previewId"].as_u64();
    let mut test = MicTest {
        connecting: (status["state"] == "connecting").then_some(status["stillConnecting"] == true),
        live: status["state"] == "active",
        vad_unavailable: id.is_some_and(|id| vad == Some((id, sensitivity, true))),
        ..Default::default()
    };
    if status["state"] == "error" {
        test.error = Some(clip(
            status["message"]
                .as_str()
                .unwrap_or("Microphone preview stopped."),
            512,
        ));
        test.retry = id.is_none();
    }
    if let Some(failure) = failure {
        test.error = Some(clip(failure, 512));
        test.retry = true;
    }
    test
}

/// Smart Auto feedback and the instant it stops being current. `selectable`
/// is the shared selector's verdict on the current inventory: it accounts for
/// Continuity permission, the built-in microphone and a closed lid.
pub fn smart_auto(
    settings: &Value,
    devices: &[Device],
    selectable: bool,
    status: Result<Value, String>,
    retry_failed: bool,
) -> (SmartAuto, Option<Duration>) {
    if settings["smartAutoMicrophoneEnabled"] != true {
        return (SmartAuto::Inactive, None);
    }
    let Ok(status) = status else {
        return (SmartAuto::Unavailable, None);
    };
    let included = settings["smartAutoApprovedDeviceIds"]
        .as_array()
        .is_some_and(|ids| !ids.is_empty());
    let after = |ms: &Value| ms.as_u64().map(Duration::from_millis);
    match status["state"].as_str() {
        Some("ready") => (SmartAuto::Inactive, after(&status["validForMs"])),
        Some("probing") => {
            let device = status["deviceId"]
                .as_str()
                .and_then(|id| devices.iter().find(|d| d.id == id))
                .map_or("included microphone", |d| d.label.as_str());
            let phase = match status["phase"].as_str() {
                Some("connecting") => "connecting",
                Some("verifying") => "checking signal",
                Some("stopping") => "finishing",
                _ => return (SmartAuto::Unavailable, None),
            };
            (
                SmartAuto::Probing {
                    device: clip(device, 256),
                    phase,
                },
                after(&status["remainingMs"]),
            )
        }
        Some("blocked") => {
            let message = if !included {
                "Include at least one microphone before Smart Auto can record."
            } else if !selectable {
                "None of the included microphones are available right now."
            } else {
                status["message"].as_str().unwrap_or_default()
            };
            let cooldown = status["retryAfterMs"].as_u64();
            (
                SmartAuto::Blocked {
                    message: clip(message, 1024),
                    cooldown_secs: cooldown.map(|ms| ms.div_ceil(1000).max(1) as u32),
                    retry: settings["smartAutoProbeEnabled"] == true && selectable,
                    retry_failed,
                },
                cooldown.map(Duration::from_millis),
            )
        }
        _ => (SmartAuto::Unavailable, None),
    }
}

/// Progress label and percent, with the shipped wording.
pub fn progress(progress: Option<&Value>) -> (String, u8) {
    let Some(p) = progress else {
        return ("Preparing production capture path…".into(), 0);
    };
    let total = p["totalCycles"].as_u64().unwrap_or(0).max(1);
    let percent = (p["completedCycles"].as_u64().unwrap_or(0) * 100 / total).min(100) as u8;
    let phase = p["phase"].as_str().unwrap_or_default();
    let current = p["currentCycle"].as_u64().unwrap_or(0);
    let text = if phase == "complete" {
        "Finishing report…".into()
    } else if current == 0 {
        format!("{} for exclusive microphone access…", report::label(phase))
    } else {
        let mut text = format!("Cycle {current} of {total}");
        if !p["backend"].is_null() {
            text += &format!(" · {}", report::backend(&p["backend"]));
        }
        if p["fallbackOccurred"] == true {
            text += " · fallback";
        }
        let setup = report::step(p);
        if !setup.is_empty() {
            text += &format!(" · {setup}");
        }
        text
    };
    (clip(&text, 256), percent)
}

/// The diagnostic as Settings shows it: progress while a run is active, else the
/// selected saved run with its history and Copy/Save confirmations.
pub fn startup(check: &Check, reports: &Reports, marks: &Marks) -> Startup {
    let running = check.run.is_some();
    let shown = if running { None } else { reports.current() };
    Startup {
        progress: running.then(|| progress(check.progress.as_ref())),
        cancelling: running && check.cancelling,
        error: check.error.as_deref().map(|e| clip(e, 512)),
        summary: shown.map(report::summary).unwrap_or_default(),
        runs: if running { vec![] } else { reports.labels() },
        selected: reports.selected,
        copied: marks.is_copied(),
        saving: marks.save() == Some(Save::Saving),
        saved: marks.save() == Some(Save::Saved),
    }
}
/// Records one run's outcome and returns the report to keep. A report must
/// answer the run that was asked for. A run whose page was left is discarded.
pub fn finish_check(
    check: &Mutex<Check>,
    id: &str,
    result: Result<Value, String>,
    presented: bool,
) -> Option<Value> {
    let mut check = lock(check);
    if check.run.as_deref() != Some(id) {
        return None;
    }
    check.run = None;
    check.progress = None;
    check.cancelling = false;
    check.abort = None;
    check.visit = 0;
    if !presented {
        check.error = None;
        return None;
    }
    match result {
        Ok(report) if report["runId"] == id => {
            check.error = None;
            Some(report)
        }
        Ok(_) => {
            check.error = Some("Murmur returned an invalid microphone startup report.".into());
            None
        }
        Err(error) => {
            check.error = Some(error);
            None
        }
    }
}

/// A registered diagnostic that has not been handed to the service yet.
pub struct Run {
    id: String,
    device: String,
    visit: u64,
    abort: Arc<AtomicBool>,
    /// The folder to save a full run into, when auto-save is on.
    auto_save: Option<String>,
}
impl Run {
    /// Valid while the Performance visit that started the run is current and
    /// nobody cancelled it.
    fn live(&self, preview: &Arc<Preview>) -> Live {
        let (preview, abort, visit) = (preview.clone(), self.abort.clone(), self.visit);
        Arc::new(move || {
            preview.performance.load(Ordering::SeqCst) == visit && !abort.load(Ordering::SeqCst)
        })
    }
}
fn forget(preview: &Preview, id: &str) {
    let mut check = lock(&preview.check);
    if check.run.as_deref() == Some(id) {
        *check = Check::default();
    }
}
/// Registers the run synchronously, so leaving the page or Cancel can revoke it
/// while its settings are read. `None`: it was revoked and nothing is shown.
pub async fn prepare_check<P: Port>(p: &P) -> Result<Option<Run>, String> {
    let preview = p.preview().clone();
    let visit = preview.performance.load(Ordering::SeqCst);
    if visit == 0 {
        return Err("Open Performance settings to test microphone startup".into());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let abort = Arc::new(AtomicBool::new(false));
    {
        let mut check = lock(&preview.check);
        if check.run.is_some() {
            return Err("A microphone startup benchmark is already running".into());
        }
        *check = Check {
            run: Some(id.clone()),
            visit,
            abort: Some(abort.clone()),
            ..Default::default()
        };
    }
    let mut run = Run {
        id,
        device: String::new(),
        visit,
        abort,
        auto_save: None,
    };
    let live = run.live(&preview);
    let settings = match p.settings().await {
        Ok(settings) => settings,
        Err(error) => {
            forget(&preview, &run.id);
            return Err(error);
        }
    };
    if !live() {
        forget(&preview, &run.id);
        return Ok(None);
    }
    run.device = settings["microphone"]
        .as_str()
        .filter(|s| !s.is_empty())
        .unwrap_or("system_default")
        .to_owned();
    run.auto_save = (settings["benchmarkAutoSave"] == true).then(|| {
        settings["benchmarkOutputDir"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    });
    Ok(Some(run))
}
/// Runs the diagnostic to its outcome and records it. The request that started
/// it has returned by now; Settings reads progress and the report back.
pub async fn run_check<P: Port>(p: &P, run: Run) {
    let preview = p.preview().clone();
    let live = run.live(&preview);
    let result = p
        .effect(
            "run_microphone_startup_benchmark",
            json!({"request":{"runId":run.id,"deviceId":run.device,"cycles":CYCLES}}),
            live.clone(),
        )
        .await;
    // Leaving the page discards the outcome, as unmounting the shipped
    // component does. A start refused because the run was cancelled before
    // the service admitted it is that cancellation, not a failure.
    let left = preview.performance.load(Ordering::SeqCst) != run.visit;
    let cancelled = !live() && matches!(&result, Err(error) if error == PRESENTATION);
    let accepted = finish_check(&preview.check, &run.id, result, !left && !cancelled);
    if let Some(finished) = accepted {
        let mut error = lock(&preview.reports).add(finished.clone()).err();
        if report::full(&finished)
            && let Some(dir) = &run.auto_save
            && let Err(reason) = p
                .call(
                    "save_microphone_startup_benchmark_report",
                    json!({"report":finished,"outputDir":dir}),
                )
                .await
        {
            error = Some(format!(
                "Run completed, but the report could not be auto-saved: {reason}"
            ));
        }
        if let Some(error) = error {
            lock(&preview.check).error = Some(clip(&error, 512));
        }
    }
    p.changed();
}
pub async fn cancel_check<P: Port>(p: &P) {
    let preview = p.preview();
    let id = {
        let mut check = lock(&preview.check);
        check.error = None;
        let id = check.run.clone();
        check.cancelling = id.is_some();
        // Revokes a run the service has not admitted yet. Once admitted, the
        // service's own cancellation stops it.
        if let Some(abort) = &check.abort {
            abort.store(true, Ordering::SeqCst);
        }
        id
    };
    let Some(id) = id else {
        lock(&preview.check).error = Some("The microphone benchmark had already stopped.".into());
        return;
    };
    let result = p
        .call("cancel_microphone_startup_benchmark", json!({"runId":id}))
        .await;
    let mut check = lock(&preview.check);
    let running = check.run.as_deref() == Some(id.as_str());
    // Not accepted means the run ended on its own or has not been claimed yet.
    // The first shows its own outcome; the second is refused at admission.
    if let Err(error) = result {
        check.cancelling &= !running;
        check.error = Some(format!(
            "Could not cancel the microphone benchmark: {error}"
        ));
    }
}
/// Puts the selected run on the clipboard. The token ends its confirmation.
pub fn copy_report<P: Port>(p: &P) -> Option<u64> {
    let preview = p.preview();
    let selected = lock(&preview.reports).current().cloned()?;
    let text = serde_json::to_string_pretty(&selected).unwrap_or_default();
    match p.copy_text(&text) {
        Ok(()) => Some(lock(&preview.marks).copied()),
        Err(error) => {
            lock(&preview.check).error =
                Some(clip(&format!("Could not copy report: {error}"), 512));
            None
        }
    }
}
/// Saves the selected run through the typed command and its output rules.
pub async fn save_report<P: Port>(p: &P, live: Live) -> Option<u64> {
    let preview = p.preview();
    let selected = lock(&preview.reports).current().cloned()?;
    if lock(&preview.marks).save() == Some(Save::Saving) {
        return None;
    }
    lock(&preview.check).error = None;
    lock(&preview.marks).saving(Save::Saving);
    p.changed();
    let saved = match p.settings().await {
        Ok(settings) => {
            let dir = settings["benchmarkOutputDir"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            p.effect(
                "save_microphone_startup_benchmark_report",
                json!({"report":selected,"outputDir":dir}),
                live,
            )
            .await
        }
        Err(error) => Err(error),
    };
    match saved {
        Ok(_) => Some(lock(&preview.marks).saving(Save::Saved)),
        Err(error) => {
            lock(&preview.marks).idle_save();
            lock(&preview.check).error =
                Some(clip(&format!("Could not save report: {error}"), 512));
            None
        }
    }
}
pub fn clear_reports<P: Port>(p: &P) {
    let preview = p.preview();
    let cleared = lock(&preview.reports).clear();
    if let Err(error) = cleared {
        lock(&preview.check).error = Some(error);
    }
}

impl Backend {
    /// Starts the diagnostic and returns at once: the run outlives the request,
    /// so Settings neither blocks on it nor loses its progress and report.
    pub async fn start_microphone_check(&self) -> Result<(), String> {
        let Some(run) = prepare_check(self).await? else {
            return Ok(());
        };
        let b = self.clone();
        self.spawn(async move { run_check(&b, run).await });
        Ok(())
    }
    pub async fn cancel_microphone_check(&self) -> Result<(), String> {
        cancel_check(self).await;
        Ok(())
    }
    pub fn copy_microphone_report(&self) {
        if let Some(token) = copy_report(self) {
            self.expire_mark(token);
        }
    }
    pub async fn save_microphone_report(&self, live: Live) {
        if let Some(token) = save_report(self, live).await {
            self.expire_mark(token);
        }
    }
    pub fn select_microphone_report(&self, index: usize) {
        lock(&self.preview.reports).select(index);
    }
    pub fn clear_microphone_reports(&self) {
        clear_reports(self);
    }
    /// Ends a Copied or Saved confirmation after the shipped 1.6 seconds.
    fn expire_mark(&self, token: u64) {
        let b = self.clone();
        self.spawn(async move {
            tokio::time::sleep(Duration::from_millis(report::FEEDBACK_MS)).await;
            if lock(&b.preview.marks).expire(token) {
                b.changed();
            }
        });
    }
}
