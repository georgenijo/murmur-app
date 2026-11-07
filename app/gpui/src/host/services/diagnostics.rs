//! Read-only Diagnostics projections. Private capture content has a separate
//! lane; its bytes never enter events, clipboard formatting or public errors.
use super::{backend::Backend, os::Live};
use crate::diagnostics::{
    host::*,
    inbox::Inbox,
    model::*,
    reports::{ImportError, Report, Source},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::Read,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
#[derive(Default)]
pub struct Clocks {
    revision: AtomicU64,
    epochs: Mutex<BTreeMap<Lane, u64>>,
    gate: tokio::sync::Mutex<()>,
    latency: Mutex<Vec<Transition>>,
}
#[derive(Clone)]
struct Transition {
    from: &'static str,
    to: &'static str,
    commit: f64,
    first: f64,
    painted: f64,
}
impl Clocks {
    fn next(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::SeqCst) + 1
    }
    pub fn epoch(&self, lane: Lane) -> u64 {
        *self
            .epochs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&lane)
            .unwrap_or(&0)
    }
    fn clear(&self, lane: Lane) -> u64 {
        let mut e = self.epochs.lock().unwrap_or_else(|p| p.into_inner());
        let current = e.entry(lane).or_default();
        *current += 1;
        if lane == Lane::Latency {
            self.latency
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clear();
        }
        *current
    }
    pub fn transition(
        &self,
        epoch: u64,
        from: &'static str,
        to: &'static str,
        commit: f64,
        first: f64,
        painted: f64,
    ) {
        let epochs = self.epochs.lock().unwrap_or_else(|p| p.into_inner());
        if epochs.get(&Lane::Latency).copied().unwrap_or(0) != epoch
            || ![commit, first, painted]
                .iter()
                .all(|v| v.is_finite() && *v >= 0.)
        {
            return;
        }
        let mut samples = self.latency.lock().unwrap_or_else(|p| p.into_inner());
        if samples.len() == 200 {
            samples.remove(0);
        }
        samples.push(Transition {
            from,
            to,
            commit,
            first,
            painted,
        });
    }
    fn latency(&self, version: &str) -> Vec<Latency> {
        let mut groups: BTreeMap<(&str, &str), Vec<Transition>> = BTreeMap::new();
        for sample in self
            .latency
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
        {
            groups
                .entry((sample.from, sample.to))
                .or_default()
                .push(sample.clone());
        }
        fn percentile(mut values: Vec<f64>, p: usize) -> Measurement {
            values.sort_by(f64::total_cmp);
            Measurement::Measured(values[((values.len() - 1) * p).div_ceil(100)])
        }
        groups
            .into_iter()
            .map(|((from, to), samples)| Latency {
                build: format!("{version} · GPUI"),
                from: from.into(),
                to: to.into(),
                count: samples.len() as u64,
                first_frame: percentile(samples.iter().map(|s| s.first).collect(), 50),
                commit: percentile(samples.iter().map(|s| s.commit).collect(), 50),
                painted: percentile(samples.iter().map(|s| s.painted).collect(), 50),
                p95: percentile(samples.iter().map(|s| s.painted).collect(), 95),
            })
            .collect()
    }
}
fn text(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().into()
}
fn number(v: &Value, k: &str) -> u64 {
    v[k].as_u64().unwrap_or(0)
}
fn measurement(v: &Value) -> Measurement {
    match v["status"].as_str() {
        Some("measured") => v["value"]
            .as_f64()
            .filter(|v| v.is_finite())
            .map(Measurement::Measured)
            .unwrap_or(Measurement::Unavailable("sample failed")),
        Some("notApplicable") => Measurement::NotApplicable,
        _ => Measurement::Unavailable("no sample"),
    }
}
fn event(v: &Value, sequence: u64) -> Option<EventRow> {
    let row = EventRow {
        sequence,
        timestamp: text(v, "timestamp"),
        stream: Stream::ALL
            .into_iter()
            .find(|s| s.label() == v["stream"].as_str().unwrap_or_default())?,
        level: Level::ALL
            .into_iter()
            .find(|s| s.label() == v["level"].as_str().unwrap_or_default())?,
        summary: text(v, "summary"),
        data: v["data"].clone(),
    };
    row.valid().then_some(row)
}
fn capture(v: &Value, detail: bool) -> PrivateCapture {
    let sections = if detail {
        ["rawText", "finalText"]
            .into_iter()
            .filter_map(|k| {
                v["result"][k]["text"]
                    .as_str()
                    .filter(|s| s.len() <= 8192)
                    .map(|s| (k.to_owned(), s.to_owned()))
            })
            .collect()
    } else {
        vec![]
    };
    PrivateCapture {
        id: text(v, "captureId"),
        recording_id: number(v, "recordingId"),
        created_ms: number(v, "capturedAtMs"),
        expires_ms: number(v, "expiresAtMs"),
        uploaded: false,
        sections,
    }
}
fn run(v: &Value) -> Option<Run> {
    let id = v["runId"].as_str()?.to_owned();
    let kind = match v["kind"].as_str()? {
        "dictation" => RunKind::Dictation,
        "fileTranscription" => RunKind::FileTranscription,
        "selectedTextTransform" => RunKind::SelectedTextTransform,
        "voiceQuery" => RunKind::VoiceQuery,
        _ => return None,
    };
    let outcome = match v["outcome"]["status"]
        .as_str()
        .or_else(|| v["outcome"].as_str())
    {
        Some("success") => Outcome::Success,
        Some("noSpeech") => Outcome::NoSpeech,
        Some("cancelled") => Outcome::Cancelled,
        Some("timedOut") => Outcome::TimedOut,
        Some("interrupted") => Outcome::Interrupted,
        _ => Outcome::Failed,
    };
    let elapsed = number(v, "finishedAtMs").saturating_sub(number(v, "startedAtMs")) as f64;
    let mut facts = Vec::new();
    for key in ["appVersion", "production", "followUps"] {
        if let Some(value) = v.get(key) {
            let s = value.to_string();
            if s.len() <= 2048 {
                facts.push((key.into(), s));
            }
        }
    }
    let mut resources = Vec::new();
    for (label, unit, r) in [
        ("Host CPU", "%", &v["resources"]["host"]["cpuPercent"]),
        (
            "Main CPU",
            "%",
            &v["resources"]["mainProcess"]["cpuPercent"],
        ),
        (
            "Main RSS",
            "bytes",
            &v["resources"]["mainProcess"]["rssBytes"],
        ),
        (
            "Rust heap",
            "bytes",
            &v["resources"]["mainProcess"]["rustHeapBytes"],
        ),
        (
            "Native heap",
            "bytes",
            &v["resources"]["mainProcess"]["ffiNativeHeapBytes"],
        ),
        (
            "Sidecar RSS",
            "bytes",
            &v["resources"]["sidecarProcess"]["rssBytes"],
        ),
    ] {
        resources.push(ResourceRange {
            label: label.into(),
            unit,
            start: measurement(&r["start"]),
            average: measurement(&r["average"]),
            peak: measurement(&r["peak"]),
            end: measurement(&r["end"]),
        });
    }
    let correlation = if let Some(id) = v["correlation"]["recordingId"].as_u64() {
        (CorrelationField::Recording, id.to_string())
    } else if let Some(id) = v["correlation"]["fileRunId"].as_u64() {
        (CorrelationField::FileRun, id.to_string())
    } else {
        (CorrelationField::Run, id.clone())
    };
    Some(Run {
        id,
        kind,
        finished_ms: number(v, "finishedAtMs"),
        outcome,
        correlation,
        runtime: v["runtimes"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| {
                ["modelId", "backend", "accelerator"]
                    .into_iter()
                    .filter_map(|k| r[k].as_str())
                    .collect::<Vec<_>>()
                    .join(" · ")
            })
            .collect::<Vec<_>>()
            .join("; "),
        input: format!(
            "Audio: {}",
            measurement(&v["input"]["audioDurationMs"]).display("ms")
        ),
        latency: Measurement::Measured(elapsed),
        rate: v["input"]["audioDurationMs"]["value"]
            .as_f64()
            .filter(|_| elapsed > 0.)
            .map(|audio| Measurement::Measured(audio / elapsed))
            .unwrap_or(Measurement::NotApplicable),
        rate_unit: "× real time",
        resources,
        stages: v["stages"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| Stage {
                name: text(s, "stage"),
                outcome: text(s, "outcome"),
                duration: measurement(&s["durationMs"]),
            })
            .collect(),
        facts,
    })
}
async fn read(b: &Backend, lane: Lane) -> Result<Payload, String> {
    match lane {
        Lane::Events => {
            let rows = b.call("get_event_history", json!({})).await?;
            Ok(Payload::Events(
                rows.as_array()
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .filter_map(|(i, v)| event(v, i as u64 + 1))
                    .collect(),
            ))
        }
        Lane::Performance => {
            let runs = b
                .call("list_performance_runs", json!({"limit":200}))
                .await?;
            let samples = b.call("get_performance_resource_window", json!({})).await?;
            Ok(Payload::Performance {
                runs: runs["runs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(run)
                    .collect(),
                samples: samples
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|s| Sample {
                        observed_ms: number(s, "observedAtMs"),
                        host_cpu: measurement(&s["host"]["cpuPercent"]),
                        main_cpu: measurement(&s["mainProcess"]["cpuPercent"]),
                        sidecar_cpu: measurement(&s["sidecarProcess"]["cpuPercent"]),
                        rss: measurement(&s["mainProcess"]["rssBytes"]),
                        rust_heap: measurement(&s["mainProcess"]["rustHeapBytes"]),
                        native_heap: measurement(&s["mainProcess"]["ffiNativeHeapBytes"]),
                        sidecar_rss: measurement(&s["sidecarProcess"]["rssBytes"]),
                    })
                    .collect(),
            })
        }
        Lane::Health => {
            let v = b.call("get_performance_store_health", json!({})).await?;
            Ok(Payload::Health(Health {
                available: v["status"] == "available",
                recommended: match v["recommendedAction"].as_str() {
                    Some("freeDisk") => Recovery::FreeDisk,
                    Some("checkPermissions") => Recovery::CheckPermissions,
                    Some("reinitializeStore") => Recovery::Reinitialize,
                    Some("restartApp") => Recovery::Restart,
                    Some("none") => Recovery::None,
                    _ => Recovery::Retry,
                },
                problem: match v["lastFailure"]["errorClass"].as_str() {
                    Some("corruptIntegrity") => Some(StoreProblem::CorruptIntegrity),
                    Some("invalidRecord") => Some(StoreProblem::InvalidRecord),
                    Some("schemaMigration") => Some(StoreProblem::SchemaMigration),
                    _ => None,
                },
                skipped: number(&v, "skippedRunCount"),
                recent_write_failed: v["lastFailure"].is_object(),
                recovered: v["lastRecovery"].is_object(),
            }))
        }
        Lane::Reports => {
            let snapshot = b.call("completion_snapshot", json!({})).await?;
            Ok(Payload::Reports(
                snapshot["reports"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| {
                        Report::parse(&serde_json::to_vec(r).ok()?, 0, Source::Local).ok()
                    })
                    .collect(),
            ))
        }
        Lane::Captures => {
            let rows = b
                .call("list_dictation_diagnostic_captures", json!({}))
                .await?;
            Ok(Payload::Captures(
                rows.as_array()
                    .into_iter()
                    .flatten()
                    .map(|v| capture(v, false))
                    .collect(),
            ))
        }
        Lane::Latency => Ok(Payload::Latency(
            b.diagnostics.latency(b.services.version()),
        )),
        Lane::Route => Ok(Payload::Route(Tab::Events)),
    }
}
impl Host for Backend {
    fn subscribe(
        &self,
        c: murmur_core::events::Consumer,
        a: u64,
        inbox: Inbox,
    ) -> Box<dyn Subscription> {
        let (send, mut receive) = tokio::sync::mpsc::channel(1);
        let clocks = self.diagnostics.clone();
        let stop = self.events.listen(
            c,
            Arc::new(move |event, _| {
                if event == "performance-diagnostics-cleared" {
                    clocks.clear(Lane::Performance);
                    clocks.clear(Lane::Health);
                }
                if matches!(
                    event,
                    "app-event"
                        | "performance-run-completed"
                        | "performance-resource-sample"
                        | "performance-diagnostics-cleared"
                        | "native-services-changed"
                ) {
                    let _ = send.try_send(());
                }
            }),
        );
        let b = self.clone();
        self.spawn(async move {
            while receive.recv().await.is_some() && !inbox.is_closed() {
                let _guard = b.diagnostics.gate.lock().await;
                for lane in [
                    Lane::Events,
                    Lane::Performance,
                    Lane::Health,
                    Lane::Captures,
                ] {
                    let epoch = b.diagnostics.epoch(lane);
                    if let Ok(payload) = read(&b, lane).await
                        && !inbox.is_closed()
                        && b.diagnostics.epoch(lane) == epoch
                    {
                        let _ = inbox.publish(Event::Notification {
                            attachment: a,
                            lane,
                            revision: b.diagnostics.next(),
                            store_epoch: epoch,
                            payload,
                        });
                    }
                }
            }
        });
        Box::new(stop)
    }
    fn request(&self, r: Request, inbox: Inbox) {
        if !r.lease.is_current() || inbox.is_closed() {
            return;
        }
        let b = self.clone();
        let current: Live = Arc::new(move || r.lease.is_current());
        self.spawn(async move {
            let _guard = b.diagnostics.gate.lock().await;
            if !current() {
                return;
            }
            let result: Result<Event, String> = async {
                let revision = b.diagnostics.next();
                Ok(match r.kind {
                    RequestKind::Read => Event::Read {
                        ticket: r.ticket,
                        revision,
                        payload: read(&b, r.ticket.lane).await?,
                    },
                    RequestKind::CopyEvents(text) | RequestKind::CopyLatency(text) => {
                        let epochs = b
                            .diagnostics
                            .epochs
                            .lock()
                            .unwrap_or_else(|p| p.into_inner());
                        if current()
                            && epochs.get(&r.ticket.lane).copied().unwrap_or(0)
                                == r.ticket.store_epoch
                        {
                            b.services.copy_text(&text)?;
                        }
                        Event::Action {
                            ticket: r.ticket,
                            success: true,
                        }
                    }
                    RequestKind::ClearEvents
                    | RequestKind::ClearPerformance
                    | RequestKind::ClearLatency => {
                        let command = match r.ticket.lane {
                            Lane::Events => Some("clear_event_history"),
                            Lane::Performance => Some("clear_performance_diagnostics"),
                            _ => None,
                        };
                        if let Some(command) = command {
                            b.effect(command, json!({}), current.clone()).await?;
                        }
                        Event::Cleared {
                            attachment: r.ticket.attachment,
                            lane: r.ticket.lane,
                            revision,
                            store_epoch: b.diagnostics.clear(r.ticket.lane),
                        }
                    }
                    RequestKind::Recover { allow_reinitialize } => {
                        b.effect(
                            "recover_performance_store",
                            json!({"allowReinitialize":allow_reinitialize}),
                            current.clone(),
                        )
                        .await?;
                        Event::Read {
                            ticket: r.ticket,
                            revision,
                            payload: read(&b, Lane::Health).await?,
                        }
                    }
                    RequestKind::ImportReport => {
                        let result = async {
                            let Some(path) = b
                                .panel(
                                    json!({"kind":"open","extensions":["json"]}),
                                    current.clone(),
                                )
                                .await
                                .map_err(|_| ImportError::ReadFailed)?
                            else {
                                return Ok(None);
                            };
                            if !current() {
                                return Ok(None);
                            }
                            let file =
                                std::fs::File::open(path).map_err(|_| ImportError::ReadFailed)?;
                            let mut bytes = Vec::new();
                            file.take(crate::diagnostics::reports::MAX_BYTES as u64 + 1)
                                .read_to_end(&mut bytes)
                                .map_err(|_| ImportError::ReadFailed)?;
                            Report::parse(&bytes, 0, Source::Imported).map(Some)
                        }
                        .await;
                        Event::Import {
                            ticket: r.ticket,
                            result,
                        }
                    }
                    RequestKind::ReadCapture { capture_id } => {
                        let capture_data = b
                            .effect(
                                "get_dictation_diagnostic_capture",
                                json!({"captureId":capture_id}),
                                current.clone(),
                            )
                            .await?;
                        Event::Read {
                            ticket: r.ticket,
                            revision,
                            payload: Payload::CaptureDetail(capture(&capture_data, true)),
                        }
                    }
                    RequestKind::DeleteCapture { capture_id } => {
                        b.effect(
                            "delete_dictation_diagnostic_capture",
                            json!({"captureId":capture_id}),
                            current.clone(),
                        )
                        .await?;
                        Event::Read {
                            ticket: r.ticket,
                            revision,
                            payload: read(&b, Lane::Captures).await?,
                        }
                    }
                    RequestKind::UploadCapture { capture_id } => {
                        b.effect(
                            "upload_dictation_diagnostic_capture",
                            json!({"captureId":capture_id}),
                            current.clone(),
                        )
                        .await?;
                        Event::Action {
                            ticket: r.ticket,
                            success: true,
                        }
                    }
                })
            }
            .await;
            if current()
                && !inbox.is_closed()
                && (matches!(&result, Ok(Event::Cleared { .. }))
                    || b.diagnostics.epoch(r.ticket.lane) == r.ticket.store_epoch)
            {
                let _ = inbox.publish(result.unwrap_or(Event::Read {
                    ticket: r.ticket,
                    revision: b.diagnostics.next(),
                    payload: Payload::Failed,
                }));
            }
        });
    }
}
