//! A revocable Settings-page meter. Capture admission and PCM disposal belong
//! to the existing preview owner; no monitor is started by a subscription.
use super::backend::Backend;
use super::os::Live;
use crate::settings::model::{Device, MicTest, SmartAuto, Startup};
use serde_json::{Value, json};
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
const CYCLES: u8 = 5;

pub fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

#[derive(Default)]
pub struct Preview {
    epoch: AtomicU64,
    pub id: AtomicU64,
    pub meter: Mutex<Option<(u64, f32, f32)>>,
    /// Latest voice-detection decision: (preview id, sensitivity, unavailable).
    pub vad: Mutex<Option<(u64, u32, bool)>>,
    /// A start or stop the service refused. Automatic starts stay suspended
    /// until the configuration changes or the user retries.
    pub failure: Mutex<Option<String>>,
    pub retry: AtomicBool,
    pub probe_retry_failed: AtomicBool,
    /// When the Smart Auto status last projected stops being current.
    pub smart_deadline: Mutex<Option<Instant>>,
    pub check: Mutex<Check>,
}
impl Preview {
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
                let next = Some((
                    id,
                    sensitivity.min(100) as u32,
                    value["decision"] == "unavailable",
                ));
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
/// One microphone startup diagnostic and its outcome. The report outlives the
/// run so Settings can show it after the request that started it returned.
#[derive(Default)]
pub struct Check {
    pub run: Option<String>,
    pub progress: Option<Value>,
    pub cancelling: bool,
    pub report: Option<Value>,
    pub error: Option<String>,
}

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
        self.events.publish("main", CHANGED, "{}");
    }
    pub fn preview_page(&self, page: Option<crate::settings::schema::Page>, _attachment: u64) {
        use crate::settings::schema::Page;
        let epoch = self.preview.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.preview.id.store(0, Ordering::SeqCst);
        self.services.main_hidden();
        if page.is_none() {
            self.navigate(super::backend::Route::Calibration {
                offset: None,
                finish: true,
            });
        }
        if page != Some(Page::Performance) {
            // Leaving the diagnostic's page stops its run, as the shipped
            // component does on unmount.
            let run = lock(&self.preview.check).run.clone();
            if let Some(run) = run {
                let b = self.clone();
                self.spawn(async move {
                    let _ = b
                        .call("cancel_microphone_startup_benchmark", json!({"runId":run}))
                        .await;
                });
            }
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
async fn tick(b: &Backend, live: &Live, state: &mut Loop) {
    let Ok(settings) = super::projections::settings(b).await else {
        return;
    };
    if !live()
        || !b
            .call("check_microphone_permission_status", json!({}))
            .await
            .is_ok_and(|v| v == "granted")
        || !live()
    {
        return;
    }
    let Ok(status) = b.call("get_microphone_preview_status", json!({})).await else {
        return;
    };
    if !live() {
        return;
    }
    let (want, sensitivity) = (config(&settings), vad(&settings));
    let retry = b.preview.retry.swap(false, Ordering::SeqCst);
    let step = state.step(&want, sensitivity, &status, retry);
    if (retry || state.suspended.is_none()) && lock(&b.preview.failure).take().is_some() {
        b.changed();
    }
    if status["previewId"].is_null() {
        b.preview.id.store(0, Ordering::SeqCst);
    }
    match step {
        Step::Wait => {}
        Step::Start => {
            match b
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
                            b.preview.id.store(id, Ordering::SeqCst);
                            state.started(id, want, sensitivity);
                        } else {
                            let _ = b
                                .call("cancel_microphone_preview", json!({"previewId":id}))
                                .await;
                        }
                    }
                }
                Err(error) if live() && !error.contains(DICTATION_BUSY) => {
                    state.suspended = Some(want);
                    *lock(&b.preview.failure) = Some(clip(&error, 512));
                    b.changed();
                }
                Err(_) => {}
            }
        }
        Step::Stop(id) => {
            match b
                .call("stop_microphone_preview", json!({"previewId":id}))
                .await
            {
                Ok(_) => {
                    state.stopped();
                    b.preview.id.store(0, Ordering::SeqCst);
                }
                Err(error) if live() => {
                    state.suspended = Some(want);
                    *lock(&b.preview.failure) = Some(clip(&error, 512));
                    b.changed();
                }
                Err(_) => {}
            }
        }
        Step::Vad(id, value) => {
            let updated = b
                .call(
                    "update_microphone_preview_vad_sensitivity",
                    json!({"previewId":id,"vadSensitivity":value}),
                )
                .await;
            // Reassert nothing on failure: the shipped pane reports it once.
            state.sent(value);
            let mut vad = lock(&b.preview.vad);
            *vad = match updated {
                Ok(_) => None,
                Err(_) => Some((id, value, true)),
            };
            drop(vad);
            b.changed();
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

/// Smart Auto feedback and the instant it stops being current.
pub fn smart_auto(
    settings: &Value,
    devices: &[Device],
    status: Result<Value, String>,
    retry_failed: bool,
) -> (SmartAuto, Option<Duration>) {
    if settings["smartAutoMicrophoneEnabled"] != true {
        return (SmartAuto::Inactive, None);
    }
    let Ok(status) = status else {
        return (SmartAuto::Unavailable, None);
    };
    let approved: Vec<&str> = settings["smartAutoApprovedDeviceIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect();
    let available = devices
        .iter()
        .any(|d| d.available && approved.contains(&d.id.as_str()));
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
            let message = if approved.is_empty() {
                "Include at least one microphone before Smart Auto can record."
            } else if !available {
                "None of the included microphones are available right now."
            } else {
                status["message"].as_str().unwrap_or_default()
            };
            let cooldown = status["retryAfterMs"].as_u64();
            (
                SmartAuto::Blocked {
                    message: clip(message, 1024),
                    cooldown_secs: cooldown.map(|ms| ms.div_ceil(1000).max(1) as u32),
                    retry: settings["smartAutoProbeEnabled"] == true && available,
                    retry_failed,
                },
                cooldown.map(Duration::from_millis),
            )
        }
        _ => (SmartAuto::Unavailable, None),
    }
}

fn label(value: &str) -> String {
    let words = value.replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
fn backend(value: &Value) -> &'static str {
    match value.as_str() {
        Some("auhal") => "AUHAL",
        Some(_) => "IOProc",
        None => "—",
    }
}
fn ms(value: Option<f64>) -> String {
    value.map_or("—".into(), |v| format!("{} ms", v.round() as i64))
}
fn step(value: &Value) -> String {
    match value["lastSetupStep"].as_str() {
        Some(step) => format!(
            "{} {}",
            label(step),
            value["lastSetupTransition"].as_str().unwrap_or_default()
        )
        .trim_end()
        .into(),
        None => String::new(),
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
        format!("{} for exclusive microphone access…", label(phase))
    } else {
        let mut text = format!("Cycle {current} of {total}");
        if !p["backend"].is_null() {
            text += &format!(" · {}", backend(&p["backend"]));
        }
        if p["fallbackOccurred"] == true {
            text += " · fallback";
        }
        let setup = step(p);
        if !setup.is_empty() {
            text += &format!(" · {setup}");
        }
        text
    };
    (clip(&text, 256), percent)
}

/// The benchmark report as bounded display lines: totals first, then cycles.
pub fn summary(report: &Value) -> Vec<String> {
    let cycles = report["cycles"].as_array().map_or(&[][..], Vec::as_slice);
    let ready: Vec<&Value> = cycles.iter().filter(|c| c["outcome"] == "ready").collect();
    let mut latencies: Vec<f64> = ready
        .iter()
        .filter_map(|c| c["cycleStartToFirstPcmMs"].as_f64())
        .collect();
    latencies.sort_by(f64::total_cmp);
    let n = latencies.len();
    let median = match n {
        0 => None,
        n if n % 2 == 0 => Some((latencies[n / 2 - 1] + latencies[n / 2]) / 2.),
        n => Some(latencies[n / 2]),
    };
    let p95 = (n > 0).then(|| {
        latencies[((n as f64 * 0.95).ceil() as usize)
            .saturating_sub(1)
            .min(n - 1)]
    });
    let wins = |auhal: bool| {
        ready
            .iter()
            .filter(|c| (c["backend"] == "auhal") == auhal && !c["backend"].is_null())
            .count()
    };
    let mut lines = vec![
        clip(
            &format!(
                "Startup results · {}/{} cycles{} · macOS · Murmur v{}",
                report["completedCycles"],
                report["requestedCycles"],
                if report["cancelled"] == true {
                    " · cancelled early"
                } else {
                    ""
                },
                report["appVersion"].as_str().unwrap_or_default()
            ),
            512,
        ),
        format!(
            "Median {} · P95 {} · Ready {}/{} · Fallbacks {}",
            ms(median),
            ms(p95),
            ready.len(),
            report["completedCycles"],
            cycles
                .iter()
                .filter(|c| c["fallbackOccurred"] == true)
                .count()
        ),
        format!(
            "AUHAL won {}; IOProc won {}. Range {}–{}.",
            wins(true),
            wins(false),
            ms(latencies.first().copied()),
            ms(latencies.last().copied())
        ),
    ];
    if cycles.is_empty() {
        lines.push("The run was cancelled before its first cycle completed.".into());
    }
    for c in cycles.iter().take(24) {
        let mut line = format!(
            "Cycle {}: {}{} · {} · {}",
            c["cycle"],
            match c["outcome"].as_str() {
                Some("ready") => "Ready",
                Some("failed") => "Failed",
                _ => "Cancelled",
            },
            if c["fallbackOccurred"] == true {
                " · fallback"
            } else {
                ""
            },
            backend(&c["backend"]),
            ms(c["cycleStartToFirstPcmMs"].as_f64())
        );
        if let Some(kind) = c["failureKind"].as_str() {
            line += &format!(" · {}", label(kind));
        }
        let setup = step(c);
        line += &format!(
            " · {}",
            if setup.is_empty() {
                "No native setup step reported".into()
            } else {
                setup
            }
        );
        lines.push(clip(&line, 512));
    }
    lines
}

pub fn startup(check: &Check) -> Startup {
    let running = check.run.is_some();
    Startup {
        progress: running.then(|| progress(check.progress.as_ref())),
        cancelling: running && check.cancelling,
        error: check.error.as_deref().map(|e| clip(e, 512)),
        summary: match (&check.report, running) {
            (Some(report), false) => summary(report),
            _ => vec![],
        },
    }
}
/// Records one run's outcome. A report must answer the run that was asked for.
pub fn finish_check(check: &Mutex<Check>, id: &str, result: Result<Value, String>) {
    let mut check = lock(check);
    if check.run.as_deref() != Some(id) {
        return;
    }
    check.run = None;
    check.progress = None;
    check.cancelling = false;
    match result {
        Ok(report) if report["runId"] == id => {
            check.error = None;
            check.report = Some(report);
        }
        Ok(_) => check.error = Some("Murmur returned an invalid microphone startup report.".into()),
        Err(error) => check.error = Some(error),
    }
}

impl Backend {
    /// Starts the diagnostic and returns at once: the run outlives the request,
    /// so Settings neither blocks on it nor loses its progress and report.
    pub async fn start_microphone_check(&self) -> Result<(), String> {
        let settings = super::projections::settings(self).await?;
        let id = uuid::Uuid::new_v4().to_string();
        {
            let mut check = lock(&self.preview.check);
            if check.run.is_some() {
                return Err("A microphone startup benchmark is already running".into());
            }
            *check = Check {
                run: Some(id.clone()),
                ..Default::default()
            };
        }
        let device = settings["microphone"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("system_default")
            .to_owned();
        let b = self.clone();
        self.spawn(async move {
            let result = b
                .effect(
                    "run_microphone_startup_benchmark",
                    json!({"request":{"runId":id,"deviceId":device,"cycles":CYCLES}}),
                    Arc::new(|| true),
                )
                .await;
            finish_check(&b.preview.check, &id, result);
            b.changed();
        });
        Ok(())
    }
    pub async fn cancel_microphone_check(&self) -> Result<(), String> {
        let id = {
            let mut check = lock(&self.preview.check);
            check.error = None;
            let id = check.run.clone();
            check.cancelling = id.is_some();
            id
        };
        let Some(id) = id else {
            lock(&self.preview.check).error =
                Some("The microphone benchmark had already stopped.".into());
            return Ok(());
        };
        let result = self
            .call("cancel_microphone_startup_benchmark", json!({"runId":id}))
            .await;
        let mut check = lock(&self.preview.check);
        let running = check.run.as_deref() == Some(id.as_str());
        match result {
            Ok(accepted) if accepted == true => {}
            // The run ended by itself first; its own outcome is already shown.
            Ok(_) if !running => {}
            Ok(_) => {
                check.cancelling = false;
                check.error = Some("The microphone benchmark had already stopped.".into());
            }
            Err(error) => {
                check.cancelling &= !running;
                check.error = Some(format!(
                    "Could not cancel the microphone benchmark: {error}"
                ));
            }
        }
        Ok(())
    }
}
