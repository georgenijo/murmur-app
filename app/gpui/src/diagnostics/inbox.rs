//! Bounded ordered handoff. Backpressure is explicit; detached producers cannot
//! publish into a successor view. No payload logging, serde or Debug.
use super::{model::*, reports};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
#[derive(Default, Clone)]
pub struct Inbox(Arc<Mutex<Pending>>);
#[derive(Default)]
struct Pending {
    queue: VecDeque<(Event, usize)>,
    bytes: usize,
    closed: bool,
}
impl Inbox {
    pub fn publish(&self, event: Event) -> Result<(), Box<Event>> {
        let bytes = cost(&event);
        let mut p = self.0.lock().unwrap_or_else(|p| p.into_inner());
        if p.closed
            || p.queue.len() >= 64
            || bytes > 96 * 1024 * 1024
            || p.bytes.saturating_add(bytes) > 96 * 1024 * 1024
        {
            return Err(Box::new(event));
        }
        p.bytes += bytes;
        p.queue.push_back((event, bytes));
        Ok(())
    }
    pub(crate) fn pending(&self) -> bool {
        !self
            .0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .queue
            .is_empty()
    }
    pub fn is_closed(&self) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).closed
    }
    pub(crate) fn close(&self) {
        let mut p = self.0.lock().unwrap_or_else(|p| p.into_inner());
        p.closed = true;
        p.queue.clear();
        p.bytes = 0;
    }
    pub(crate) fn drain(&self) -> VecDeque<Event> {
        let mut p = self.0.lock().unwrap_or_else(|p| p.into_inner());
        p.bytes = 0;
        std::mem::take(&mut p.queue)
            .into_iter()
            .map(|(e, _)| e)
            .collect()
    }
}
fn cost(event: &Event) -> usize {
    let fail = usize::MAX;
    let run_cost = |r: &Run| {
        512 + r.id.len()
            + r.correlation.1.len()
            + r.runtime.len()
            + r.input.len()
            + r.stages
                .iter()
                .map(|s| s.name.len() + s.outcome.len() + 64)
                .sum::<usize>()
            + r.facts
                .iter()
                .map(|(k, v)| k.len() + v.len() + 64)
                .sum::<usize>()
            + r.resources
                .iter()
                .map(|r| r.label.len() + 512)
                .sum::<usize>()
    };
    match event {
        Event::Read { payload, .. } | Event::Notification { payload, .. } => match payload {
            Payload::Events(v) if v.len() <= MAX_EVENTS => v
                .iter()
                .map(|e| e.summary.len() + e.timestamp.len() + e.data.to_string().len() + 128)
                .sum(),
            Payload::Event(e) => {
                e.summary.len() + e.timestamp.len() + e.data.to_string().len() + 128
            }
            Payload::Performance { runs, samples }
                if runs.len() <= MAX_RUNS && samples.len() <= MAX_SAMPLES =>
            {
                runs.iter().map(run_cost).sum::<usize>() + samples.len() * 512
            }
            Payload::Run(r) => run_cost(r),
            Payload::Sample(_) | Payload::Health(_) | Payload::Route(_) | Payload::Failed => 512,
            Payload::Reports(v) if v.len() <= 10 => {
                v.iter().map(reports::Report::retained_bytes).sum()
            }
            Payload::Latency(v) if v.len() <= MAX_SAMPLES => v
                .iter()
                .map(|l| l.build.len() + l.from.len() + l.to.len() + 256)
                .sum(),
            Payload::CaptureDetail(c) => {
                256 + c.id.len()
                    + c.sections
                        .iter()
                        .map(|(k, v)| k.len() + v.len())
                        .sum::<usize>()
            }
            Payload::Captures(v) if v.len() <= 20 => v
                .iter()
                .map(|c| {
                    256 + c.id.len()
                        + c.sections
                            .iter()
                            .map(|(k, v)| k.len() + v.len())
                            .sum::<usize>()
                })
                .sum(),
            _ => fail,
        },
        Event::Import {
            result: Ok(Some(r)),
            ..
        } => r.retained_bytes(),
        _ => 128,
    }
}
