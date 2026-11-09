//! Microphone startup reports: the wording of the shipped dashboard, the saved
//! run history and the transient Copy/Save feedback. Reports contain only fixed
//! enums, timings and counters, never a device name, path or audio.
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The shipped dashboard keeps the latest ten full runs.
pub const MAX_SAVED: usize = 10;
const FILE: &str = "microphone-startup-reports.json";
const MAX_FILE_BYTES: u64 = 1 << 20;
/// Transient "Copied"/"Saved" confirmation, as in the shipped component.
pub const FEEDBACK_MS: u64 = 1600;

fn clip(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}
pub fn label(value: &str) -> String {
    let words = value.replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}
/// The service serializes the real backend enum: `auhal` or `cpal`.
pub fn backend(value: &Value) -> &'static str {
    match value.as_str() {
        Some("auhal") => "AUHAL",
        Some("cpal") => "CPAL",
        Some(_) => "Unknown backend",
        None => "—",
    }
}
pub fn ms(value: Option<f64>) -> String {
    value.map_or("—".into(), |v| format!("{} ms", v.round() as i64))
}
pub fn step(value: &Value) -> String {
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
fn median(sorted: &[f64]) -> Option<f64> {
    let n = sorted.len();
    match n {
        0 => None,
        n if n % 2 == 0 => Some((sorted[n / 2 - 1] + sorted[n / 2]) / 2.),
        n => Some(sorted[n / 2]),
    }
}
/// Nearest rank, like the shipped dashboard.
fn p95(sorted: &[f64]) -> Option<f64> {
    let n = sorted.len();
    (n > 0).then(|| {
        sorted[((n as f64 * 0.95).ceil() as usize)
            .saturating_sub(1)
            .min(n - 1)]
    })
}

/// The report as bounded display lines: totals, per-backend attempt statistics,
/// then every cycle with its backend order, attempts, budgets and failures.
pub fn summary(report: &Value) -> Vec<String> {
    let cycles = report["cycles"].as_array().map_or(&[][..], Vec::as_slice);
    let ready: Vec<&Value> = cycles.iter().filter(|c| c["outcome"] == "ready").collect();
    let mut latencies: Vec<f64> = ready
        .iter()
        .filter_map(|c| c["cycleStartToFirstPcmMs"].as_f64())
        .collect();
    latencies.sort_by(f64::total_cmp);
    let wins = |name: &str| ready.iter().filter(|c| c["backend"] == name).count();
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
            ms(median(&latencies)),
            ms(p95(&latencies)),
            ready.len(),
            report["completedCycles"],
            cycles
                .iter()
                .filter(|c| c["fallbackOccurred"] == true)
                .count()
        ),
        format!(
            "AUHAL won {}; CPAL won {}. Range {}–{}. Diagnostic cycles read the immutable production backend order and never retrain its first-PCM preference.",
            wins("auhal"),
            wins("cpal"),
            ms(latencies.first().copied()),
            ms(latencies.last().copied())
        ),
    ];
    for name in ["auhal", "cpal"] {
        let attempts: Vec<&Value> = cycles
            .iter()
            .flat_map(|c| c["attempts"].as_array().into_iter().flatten())
            .filter(|a| a["backend"] == name)
            .collect();
        let mut won: Vec<f64> = attempts
            .iter()
            .filter(|a| a["outcome"] == "ready")
            .filter_map(|a| a["attemptStartToFirstPcmMs"].as_f64())
            .collect();
        won.sort_by(f64::total_cmp);
        lines.push(format!(
            "{} attempts: {} ready · {} failed · {} total · Median {} · P95 {} · Max {}",
            backend(&Value::from(name)),
            won.len(),
            attempts.iter().filter(|a| a["outcome"] == "failed").count(),
            attempts.len(),
            ms(median(&won)),
            ms(p95(&won)),
            ms(won.last().copied())
        ));
    }
    if cycles.is_empty() {
        lines.push("The run was cancelled before its first cycle completed.".into());
    }
    for c in cycles.iter().take(24) {
        let order: Vec<&str> = c["backendOrder"]
            .as_array()
            .into_iter()
            .flatten()
            .map(backend)
            .collect();
        let mut line = format!(
            "Cycle {}: {}{} · {}",
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
        );
        if !order.is_empty() {
            line += &format!(
                " · {} · {}",
                order.join(" → "),
                if c["backendOrderSource"] == "default" {
                    "default order"
                } else {
                    "learned memo"
                }
            );
        }
        line += &format!(" · {}", ms(c["cycleStartToFirstPcmMs"].as_f64()));
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
        for a in c["attempts"].as_array().into_iter().flatten().take(6) {
            let mut line = format!(
                "Attempt P{} {} · {}",
                a["resolutionPass"],
                backend(&a["backend"]),
                a["outcome"].as_str().unwrap_or("unknown")
            );
            match (
                a["attemptStartToFirstPcmMs"].as_f64(),
                a["activeElapsedMs"].as_f64(),
            ) {
                (Some(pcm), _) => line += &format!(" in {}", ms(Some(pcm))),
                (None, None) => line += " before elapsed timing was confirmed",
                _ => {}
            }
            if let Some(active) = a["activeElapsedMs"].as_f64() {
                line += &format!(" · {} active", ms(Some(active)));
            }
            line += &format!(" · {} budget", ms(a["attemptBudgetMs"].as_f64()));
            if let Some(kind) = a["failureKind"].as_str() {
                line += &format!(" · {}", label(kind));
                if let Some(phase) = a["failurePhase"].as_str() {
                    line += &format!(" ({})", label(phase));
                }
            }
            let setup = step(a);
            if !setup.is_empty() {
                line += &format!(" · {setup}");
            }
            lines.push(clip(&line, 512));
        }
    }
    lines
}

/// Whether a run finished all its requested cycles. Only these persist.
pub fn full(report: &Value) -> bool {
    report["cancelled"] == false && report["completedCycles"] == report["requestedCycles"]
}
fn plausible(report: &Value) -> bool {
    report["schemaVersion"] == 1
        && report["runId"].is_string()
        && report["startedAt"].as_str().is_some_and(|s| s.len() <= 64)
        && report["requestedCycles"].is_u64()
        && report["completedCycles"].is_u64()
        && report["cycles"].is_array()
}
fn started(report: &Value) -> &str {
    report["startedAt"].as_str().unwrap_or_default()
}

/// Newest-first run history. Cancelled or partial runs stay in memory for this
/// launch; full runs are also written to the data directory.
#[derive(Default)]
pub struct Reports {
    items: Vec<Value>,
    pub selected: usize,
    dir: Option<PathBuf>,
}
impl Reports {
    pub fn load(dir: &Path) -> Self {
        let mut items: Vec<Value> = std::fs::metadata(dir.join(FILE))
            .ok()
            .filter(|m| m.len() <= MAX_FILE_BYTES)
            .and_then(|_| std::fs::read(dir.join(FILE)).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        items.retain(|r| plausible(r) && full(r));
        items.sort_by(|a, b| started(b).cmp(started(a)));
        items.truncate(MAX_SAVED);
        Self {
            items,
            selected: 0,
            dir: Some(dir.to_owned()),
        }
    }
    pub fn current(&self) -> Option<&Value> {
        self.items.get(self.selected)
    }
    /// Adds a run and selects it. The error means the history could not be
    /// written; the run is still shown for this launch.
    pub fn add(&mut self, report: Value) -> Result<(), String> {
        if !plausible(&report) {
            return Err("Murmur returned an invalid microphone startup report.".into());
        }
        let at = started(&report).to_owned();
        self.items.retain(|r| started(r) != at);
        self.items.push(report.clone());
        self.items.sort_by(|a, b| started(b).cmp(started(a)));
        self.items.truncate(MAX_SAVED);
        self.selected = self
            .items
            .iter()
            .position(|r| started(r) == at)
            .unwrap_or(0);
        if full(&report) {
            self.persist()
        } else {
            Ok(())
        }
    }
    pub fn select(&mut self, index: usize) {
        if index < self.items.len() {
            self.selected = index;
        }
    }
    pub fn clear(&mut self) -> Result<(), String> {
        self.items.clear();
        self.selected = 0;
        let Some(dir) = &self.dir else { return Ok(()) };
        match std::fs::remove_file(dir.join(FILE)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err("Could not clear the saved runs on this Mac.".into())
            }
            _ => Ok(()),
        }
    }
    fn persist(&self) -> Result<(), String> {
        let Some(dir) = &self.dir else { return Ok(()) };
        let kept: Vec<&Value> = self.items.iter().filter(|r| full(r)).collect();
        let bytes = serde_json::to_vec(&kept)
            .map_err(|_| "Could not save this run to the history on this Mac.".to_string())?;
        let temporary = dir.join(format!("{FILE}.tmp"));
        std::fs::create_dir_all(dir)
            .and_then(|()| std::fs::write(temporary.as_path(), bytes))
            .and_then(|()| std::fs::rename(temporary.as_path(), dir.join(FILE)))
            .map_err(|_| "Could not save this run to the history on this Mac.".to_string())
    }
    /// Selector entries: when the run started and how far it got.
    pub fn labels(&self) -> Vec<String> {
        self.items
            .iter()
            .map(|r| {
                let at = started(r).get(..19).unwrap_or_default().replace('T', " ");
                clip(
                    &format!(
                        "{at} UTC · {}/{} cycles",
                        r["completedCycles"], r["requestedCycles"]
                    ),
                    128,
                )
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Save {
    Saving,
    Saved,
}
/// The transient "Copied" and "Saving…/Saved" confirmations. Each carries the
/// token of the change that set it, so an older timer cannot clear a newer one.
#[derive(Default)]
pub struct Marks {
    token: u64,
    copy: Option<u64>,
    save: Option<(u64, Save)>,
}
impl Marks {
    pub fn copied(&mut self) -> u64 {
        self.token += 1;
        self.copy = Some(self.token);
        self.token
    }
    pub fn saving(&mut self, state: Save) -> u64 {
        self.token += 1;
        self.save = Some((self.token, state));
        self.token
    }
    pub fn idle_save(&mut self) {
        self.save = None;
    }
    /// True when the token ended a visible confirmation.
    pub fn expire(&mut self, token: u64) -> bool {
        let mut ended = false;
        if self.copy == Some(token) {
            self.copy = None;
            ended = true;
        }
        if self.save.is_some_and(|(at, _)| at == token) {
            self.save = None;
            ended = true;
        }
        ended
    }
    pub fn is_copied(&self) -> bool {
        self.copy.is_some()
    }
    pub fn save(&self) -> Option<Save> {
        self.save.map(|(_, state)| state)
    }
}
