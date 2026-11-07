//! Synthetic standalone adapter. No application data, recording, clipboard,
//! permissions, network, telemetry or production service is opened.
use super::{host::*, inbox::Inbox, model::*, reports::*};
use std::{cell::RefCell, collections::BTreeMap};
#[derive(Default)]
pub struct FixtureHost {
    revisions: RefCell<BTreeMap<Lane, u64>>,
    epochs: RefCell<BTreeMap<Lane, u64>>,
    cleared: RefCell<std::collections::BTreeSet<Lane>>,
}
struct FixtureSubscription;
impl Subscription for FixtureSubscription {
    fn stop(&mut self) {}
}
impl Host for FixtureHost {
    fn subscribe(
        &self,
        consumer: murmur_core::events::Consumer,
        _: u64,
        _: Inbox,
    ) -> Box<dyn Subscription> {
        assert_eq!(consumer, murmur_core::events::Consumer::Diagnostics);
        Box::new(FixtureSubscription)
    }
    fn request(&self, r: Request, inbox: Inbox) {
        if !r.lease.is_current() {
            return;
        }
        let mut rev = self.revisions.borrow_mut();
        let revision = rev.entry(r.ticket.lane).or_insert(0);
        *revision += 1;
        let revision = *revision;
        let event = match r.kind {
            RequestKind::Read => Event::Read {
                ticket: r.ticket,
                revision,
                payload: if self.cleared.borrow().contains(&r.ticket.lane) {
                    match r.ticket.lane {
                        Lane::Events => Payload::Events(vec![]),
                        Lane::Performance => Payload::Performance {
                            runs: vec![],
                            samples: vec![],
                        },
                        Lane::Latency => Payload::Latency(vec![]),
                        Lane::Captures => Payload::Captures(vec![]),
                        _ => payload(r.ticket.lane),
                    }
                } else {
                    payload(r.ticket.lane)
                },
            },
            RequestKind::ReadCapture { capture_id } => Event::Read {
                ticket: r.ticket,
                revision,
                payload: Payload::CaptureDetail(capture(capture_id)),
            },
            RequestKind::ImportReport => Event::Import {
                ticket: r.ticket,
                result: Report::parse(
                    include_bytes!(
                        "../../../../tests/fixtures/diagnostic-reports/benchmark-v2.json"
                    ),
                    0,
                    Source::Imported,
                )
                .map(Some),
            },
            RequestKind::ClearEvents | RequestKind::ClearPerformance => {
                self.cleared.borrow_mut().insert(r.ticket.lane);
                let mut epochs = self.epochs.borrow_mut();
                let epoch = epochs.entry(r.ticket.lane).or_insert(0);
                *epoch += 1;
                Event::Cleared {
                    attachment: r.ticket.attachment,
                    lane: r.ticket.lane,
                    revision,
                    store_epoch: *epoch,
                }
            }
            RequestKind::ClearLatency | RequestKind::DeleteCapture { .. } => {
                self.cleared.borrow_mut().insert(r.ticket.lane);
                Event::Notification {
                    attachment: r.ticket.attachment,
                    lane: r.ticket.lane,
                    revision,
                    store_epoch: 0,
                    payload: if r.ticket.lane == Lane::Latency {
                        Payload::Latency(vec![])
                    } else {
                        Payload::Captures(vec![])
                    },
                }
            }
            // No fixture action touches disk, sends data or uses the pasteboard.
            _ => Event::Action {
                ticket: r.ticket,
                success: true,
            },
        };
        assert!(inbox.publish(event).is_ok());
    }
}
pub fn payload(lane: Lane) -> Payload {
    match lane {
        Lane::Events => Payload::Events((1..=160).map(event).collect()),
        Lane::Performance => Payload::Performance {
            runs: vec![run()],
            samples: (0..24)
                .map(|i| Sample {
                    observed_ms: 1000 + i * 1000,
                    host_cpu: Measurement::Measured(12. + (i % 7) as f64),
                    main_cpu: Measurement::Measured(3. + (i % 4) as f64),
                    sidecar_cpu: Measurement::Measured(8. + (i % 5) as f64),
                    rss: Measurement::Measured((80. + i as f64) * 1048576.),
                    rust_heap: Measurement::Measured((12. + (i % 3) as f64) * 1048576.),
                    native_heap: Measurement::Unavailable("dependency pending"),
                    sidecar_rss: Measurement::Measured(64. * 1048576.),
                })
                .collect(),
        },
        Lane::Health => Payload::Health(Health {
            available: true,
            recommended: Recovery::None,
            problem: None,
            skipped: 0,
            recent_write_failed: false,
            recovered: false,
        }),
        Lane::Reports => Payload::Reports(vec![
            Report::parse(
                include_bytes!("../../../../tests/fixtures/diagnostic-reports/benchmark-v2.json"),
                0,
                Source::Local,
            )
            .ok()
            .unwrap(),
            Report::parse(
                include_bytes!("../../../../tests/fixtures/diagnostic-reports/benchmark-v2.json"),
                0,
                Source::Local,
            )
            .ok()
            .unwrap(),
        ]),
        Lane::Latency => Payload::Latency(vec![Latency {
            build: "synthetic-build".into(),
            from: "Home".into(),
            to: "Diagnostics".into(),
            count: 12,
            first_frame: Measurement::Measured(8.),
            commit: Measurement::Measured(4.),
            painted: Measurement::Measured(16.),
            p95: Measurement::Measured(20.),
        }]),
        Lane::Captures => Payload::Captures(vec![capture("synthetic-capture".into())]),
        Lane::Route => Payload::Route(Tab::Events),
    }
}
pub fn event(sequence: u64) -> EventRow {
    EventRow {
        sequence,
        timestamp: format!("12:00:{:02}", sequence % 60),
        stream: Stream::Pipeline,
        level: if sequence.is_multiple_of(11) {
            Level::Error
        } else {
            Level::Info
        },
        summary: "Synthetic pipeline stage completed".into(),
        data: serde_json::json!({"recording_id":7,"duration_ms":12,"run_id":"synthetic-run"}),
    }
}
pub fn run() -> Run {
    Run {
        id: "synthetic-run".into(),
        kind: RunKind::Dictation,
        finished_ms: 1000,
        outcome: Outcome::Success,
        correlation: (CorrelationField::Recording, "7".into()),
        runtime: "Whisper Tiny · Metal · warm".into(),
        input: "5.5 s · small".into(),
        latency: Measurement::Measured(120.),
        rate: Measurement::Measured(0.2),
        rate_unit: "RTF",
        resources: vec![ResourceRange {
            label: "Main RSS".into(),
            unit: "MB",
            start: Measurement::Measured(80.),
            average: Measurement::Measured(85.),
            peak: Measurement::Measured(90.),
            end: Measurement::Measured(82.),
        }],
        stages: vec![
            Stage {
                name: "Capture finalization".into(),
                outcome: "completed".into(),
                duration: Measurement::Measured(12.),
            },
            Stage {
                name: "Inference decode".into(),
                outcome: "completed".into(),
                duration: Measurement::Measured(100.),
            },
            Stage {
                name: "Delivery".into(),
                outcome: "completed".into(),
                duration: Measurement::Measured(8.),
            },
        ],
        facts: vec![
            ("Warm state".into(), "Warm".into()),
            ("Capture backend".into(), "AUHAL".into()),
            ("Audio duration".into(), "5500 ms".into()),
            ("Output size bucket".into(), "Small".into()),
        ],
    }
}

pub fn capture(id: String) -> PrivateCapture {
    PrivateCapture {
        id,
        recording_id: 7,
        created_ms: 1000,
        expires_ms: 604801000,
        uploaded: false,
        sections: vec![(
            "Synthetic final text".into(),
            "A synthetic fixture sentence.".into(),
        )],
    }
}
