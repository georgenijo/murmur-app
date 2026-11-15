//! Off-thread `events.jsonl` writer.
//!
//! Logging call sites hand each event to a bounded queue and return. One thread
//! drains the queue in order and writes whole lines in batches. The queue never
//! blocks the caller: when it is full the event is dropped and counted, and the
//! count is written to the file as one `Log events dropped` line the next time
//! the writer thread runs.
//!
//! There is no buffering beyond the queue. Each batch is a single `write` to the
//! file, so a process killed at any point loses at most the events still in the
//! queue; [`EventWriter::flush`] waits for the queue to drain (exit and panic
//! paths call it).

use super::AppEvent;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Events the queue holds before it starts dropping.
pub(super) const QUEUE_CAPACITY: usize = 4096;
/// Most bytes the writer thread joins into one `write`.
const MAX_BATCH_BYTES: usize = 256 * 1024;

enum Message {
    Event(Box<AppEvent>),
    /// Acknowledged, with whether the last write succeeded, once every event
    /// queued before it has been written.
    Flush(SyncSender<bool>),
    /// Runs on the writer thread once every event queued before it has been
    /// written, with nothing written meanwhile. Clear logs uses it so events
    /// queued before a clear can never land in the fresh file.
    Exclusive(Box<dyn FnOnce() + Send>),
}

#[derive(Clone)]
pub(super) struct EventWriter {
    queue: SyncSender<Message>,
    dropped: Arc<AtomicU64>,
    dropped_total: Arc<AtomicU64>,
}

/// Loss accounting for the writer thread.
#[derive(Default)]
struct Writing {
    /// Events in batches the sink rejected, not yet recorded in the file.
    failed: u64,
    last_ok: bool,
}

/// Write `batch` (`events` lines) to `sink`, ahead of it one `Log events
/// dropped` line for everything lost since the last successful write: events
/// dropped by a full queue and events in batches the sink rejected.
fn write_batch<F>(
    sink: &mut F,
    state: &mut Writing,
    dropped: &AtomicU64,
    batch: &mut Vec<u8>,
    events: &mut u64,
    force: bool,
) where
    F: FnMut(&[u8]) -> io::Result<()>,
{
    // A pending loss alone is written only when a flush or clear asks, never as
    // a silent retry after the caller has been told the last write failed.
    if batch.is_empty() && !force {
        return;
    }
    let lost = dropped.swap(0, Ordering::Relaxed) + state.failed;
    if lost > 0 || !batch.is_empty() {
        let mut out = Vec::with_capacity(batch.len() + 128);
        if lost > 0 {
            out.extend_from_slice(super::event_drop_note(lost).as_bytes());
        }
        out.extend_from_slice(batch);
        if sink(&out).is_ok() {
            state.failed = 0;
            state.last_ok = true;
        } else {
            // The note was not written either: carry its count forward.
            state.failed = lost + *events;
            state.last_ok = false;
        }
    }
    batch.clear();
    *events = 0;
}

impl EventWriter {
    /// Start the writer thread. `sink` receives each batch of whole lines.
    pub(super) fn spawn<F>(mut sink: F, capacity: usize) -> Self
    where
        F: FnMut(&[u8]) -> io::Result<()> + Send + 'static,
    {
        let (queue, receiver) = mpsc::sync_channel::<Message>(capacity);
        let dropped = Arc::new(AtomicU64::new(0));
        let dropped_total = Arc::new(AtomicU64::new(0));
        let worker_dropped = Arc::clone(&dropped);
        let _ = std::thread::Builder::new()
            .name("event-log-writer".into())
            .spawn(move || {
                let mut batch: Vec<u8> = Vec::with_capacity(16 * 1024);
                let mut events = 0u64;
                let mut state = Writing {
                    last_ok: true,
                    ..Writing::default()
                };
                while let Ok(first) = receiver.recv() {
                    let mut next = Some(first);
                    while let Some(message) = next.take() {
                        match message {
                            Message::Event(event) => {
                                if let Ok(line) = serde_json::to_string(&*event) {
                                    batch.extend_from_slice(line.as_bytes());
                                    batch.push(b'\n');
                                    events += 1;
                                }
                            }
                            Message::Flush(ack) => {
                                write_batch(
                                    &mut sink,
                                    &mut state,
                                    &worker_dropped,
                                    &mut batch,
                                    &mut events,
                                    true,
                                );
                                let _ = ack.send(state.last_ok);
                            }
                            Message::Exclusive(action) => {
                                write_batch(
                                    &mut sink,
                                    &mut state,
                                    &worker_dropped,
                                    &mut batch,
                                    &mut events,
                                    false,
                                );
                                action();
                                // The action resets the file: earlier losses
                                // belong to what it removed.
                                worker_dropped.store(0, Ordering::Relaxed);
                                state.failed = 0;
                            }
                        }
                        if batch.len() < MAX_BATCH_BYTES {
                            next = receiver.try_recv().ok();
                        }
                    }
                    write_batch(
                        &mut sink,
                        &mut state,
                        &worker_dropped,
                        &mut batch,
                        &mut events,
                        false,
                    );
                }
            });
        Self {
            queue,
            dropped,
            dropped_total,
        }
    }

    /// Queue an event without blocking. A full queue drops it and counts it.
    pub(super) fn send(&self, event: AppEvent) {
        match self.queue.try_send(Message::Event(Box::new(event))) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                self.dropped_total.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Events dropped because the queue was full, since launch.
    #[allow(dead_code)] // Read by tests; kept for diagnostics.
    pub(super) fn dropped_total(&self) -> u64 {
        self.dropped_total.load(Ordering::Relaxed)
    }

    /// Queue a control message, waiting for room until `deadline`.
    fn enqueue(&self, mut message: Message, deadline: Instant) -> bool {
        loop {
            match self.queue.try_send(message) {
                Ok(()) => return true,
                Err(TrySendError::Full(returned)) if Instant::now() < deadline => {
                    message = returned;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(_) => return false,
            }
        }
    }

    /// Wait until everything queued so far is written, or `timeout` passes.
    /// Returns whether it was written: a rejected write reports `false`.
    pub(crate) fn flush(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let (ack, acked) = mpsc::sync_channel(1);
        if !self.enqueue(Message::Flush(ack), deadline) {
            return false;
        }
        acked
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .unwrap_or(false)
    }

    /// Run `action` on the writer thread after every event queued so far has
    /// been written and before any later one. `None` if it could not be queued
    /// or did not finish within `timeout`.
    pub(super) fn run_exclusive<R, F>(&self, action: F, timeout: Duration) -> Option<R>
    where
        R: Send + 'static,
        F: FnOnce() -> R + Send + 'static,
    {
        let deadline = Instant::now() + timeout;
        let (result, received) = mpsc::sync_channel(1);
        let message = Message::Exclusive(Box::new(move || {
            let _ = result.send(action());
        }));
        if !self.enqueue(message, deadline) {
            return None;
        }
        received
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Condvar, Mutex};

    fn event(n: u64) -> AppEvent {
        AppEvent {
            timestamp: "t".into(),
            stream: "audio".into(),
            level: "info".into(),
            summary: format!("event {n}"),
            data: serde_json::json!({ "n": n }),
        }
    }

    fn numbers(written: &[u8]) -> Vec<u64> {
        String::from_utf8(written.to_vec())
            .unwrap()
            .lines()
            .filter_map(|line| serde_json::from_str::<AppEvent>(line).ok())
            .filter_map(|event| event.data["n"].as_u64())
            .collect()
    }

    #[test]
    fn events_are_written_in_order_as_whole_lines() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&written);
        let writer = EventWriter::spawn(
            move |bytes| {
                sink.lock().unwrap().extend_from_slice(bytes);
                Ok(())
            },
            QUEUE_CAPACITY,
        );
        for n in 0..2000 {
            writer.send(event(n));
        }
        assert!(writer.flush(Duration::from_secs(5)));
        let written = written.lock().unwrap();
        assert!(written.ends_with(b"\n"));
        assert_eq!(numbers(&written), (0..2000).collect::<Vec<_>>());
        assert_eq!(writer.dropped_total(), 0);
    }

    #[test]
    fn a_full_queue_drops_and_counts_without_blocking_the_caller() {
        // The sink blocks on its first write, so the queue fills behind it.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let sink_gate = Arc::clone(&gate);
        let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sink_entered = Arc::clone(&entered);
        let written = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&written);
        let writer = EventWriter::spawn(
            move |bytes| {
                sink_entered.store(true, Ordering::SeqCst);
                let (open, signal) = &*sink_gate;
                let mut open = open.lock().unwrap();
                while !*open {
                    open = signal.wait(open).unwrap();
                }
                sink.lock().unwrap().extend_from_slice(bytes);
                Ok(())
            },
            8,
        );
        // Let the first event reach the blocked sink, then flood the queue.
        writer.send(event(0));
        while !entered.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(1));
        }
        let started = Instant::now();
        for n in 1..100 {
            writer.send(event(n));
        }
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "send must never wait on the sink"
        );
        // Eight sit in the queue behind the held batch; the rest are dropped.
        assert_eq!(writer.dropped_total(), 99 - 8);

        {
            let (open, signal) = &*gate;
            *open.lock().unwrap() = true;
            signal.notify_all();
        }
        assert!(writer.flush(Duration::from_secs(5)));
        let written = String::from_utf8(written.lock().unwrap().clone()).unwrap();
        // The loss is recorded in the file as one counted line, ahead of events
        // that came after it, and what survived stays in order.
        assert!(written.contains("Log events dropped"), "{written}");
        let kept = numbers(written.as_bytes());
        assert!(kept.windows(2).all(|pair| pair[0] < pair[1]), "{kept:?}");
        assert_eq!(
            kept.len() as u64 + writer.dropped_total(),
            100,
            "every event is either written or counted"
        );
    }

    #[test]
    fn clear_logs_resets_the_file_under_the_writer_thread() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("murmur-writer-clear-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        let file = super::super::SizeRotatingFile::open(path.clone(), 1 << 20, 2).unwrap();
        let mut sink = file.clone();
        let writer = EventWriter::spawn(move |bytes| sink.write_all(bytes), QUEUE_CAPACITY);
        writer.send(event(1));
        assert!(writer.flush(Duration::from_secs(5)));
        assert_eq!(numbers(&std::fs::read(&path).unwrap()), vec![1]);

        // What Clear logs does: delete and reopen under the writer's own lock.
        super::super::reset_writers(&[file]);
        assert_eq!(std::fs::read(&path).unwrap().len(), 0);
        writer.send(event(2));
        assert!(writer.flush(Duration::from_secs(5)));
        assert_eq!(numbers(&std::fs::read(&path).unwrap()), vec![2]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn events_queued_before_a_clear_never_land_in_the_fresh_file() {
        use std::io::Write;
        let dir =
            std::env::temp_dir().join(format!("murmur-writer-pending-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        let file = super::super::SizeRotatingFile::open(path.clone(), 1 << 20, 2).unwrap();

        // The sink pauses before its first write, so later events sit queued.
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let sink_gate = Arc::clone(&gate);
        let mut sink = file.clone();
        let writer = EventWriter::spawn(
            move |bytes| {
                let (open, signal) = &*sink_gate;
                let mut open = open.lock().unwrap();
                while !*open {
                    open = signal.wait(open).unwrap();
                }
                drop(open);
                sink.write_all(bytes)
            },
            QUEUE_CAPACITY,
        );
        for n in 1..=5 {
            writer.send(event(n));
        }
        let clearing = {
            let writer = writer.clone();
            let file = file.clone();
            std::thread::spawn(move || {
                writer.run_exclusive(
                    move || super::super::reset_writers(&[file]),
                    Duration::from_secs(10),
                )
            })
        };
        // Clear is queued behind the events; releasing the sink lets them be
        // written and then removed by the reset.
        {
            let (open, signal) = &*gate;
            *open.lock().unwrap() = true;
            signal.notify_all();
        }
        assert!(clearing.join().unwrap().is_some());
        writer.send(event(99));
        assert!(writer.flush(Duration::from_secs(5)));
        assert_eq!(numbers(&std::fs::read(&path).unwrap()), vec![99]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_rejected_batch_is_counted_by_events_not_by_failed_writes() {
        let accept = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let sink_accept = Arc::clone(&accept);
        let written = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&written);
        let writer = EventWriter::spawn(
            move |bytes| {
                if sink_accept.load(Ordering::SeqCst) {
                    sink.lock().unwrap().extend_from_slice(bytes);
                    Ok(())
                } else {
                    Err(io::Error::other("injected"))
                }
            },
            QUEUE_CAPACITY,
        );
        // Three batches, one event each, all rejected; flush says so.
        for n in 0..3 {
            writer.send(event(n));
            assert!(!writer.flush(Duration::from_secs(5)));
        }
        // A batch of four rejected at once is four losses, not one.
        accept.store(false, Ordering::SeqCst);
        for n in 3..7 {
            writer.send(event(n));
        }
        assert!(!writer.flush(Duration::from_secs(5)));

        accept.store(true, Ordering::SeqCst);
        writer.send(event(100));
        assert!(writer.flush(Duration::from_secs(5)));
        let written = String::from_utf8(written.lock().unwrap().clone()).unwrap();
        let note: AppEvent = serde_json::from_str(written.lines().next().unwrap()).unwrap();
        assert_eq!(note.summary, "Log events dropped");
        assert_eq!(note.data["dropped_events"], 7, "{written}");
        assert_eq!(numbers(written.as_bytes()), vec![100]);
    }

    #[test]
    fn failed_creation_then_recovery_records_every_lost_event() {
        use std::io::Write;
        let dir =
            std::env::temp_dir().join(format!("murmur-writer-recover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events.jsonl");
        let fs = Arc::new(super::super::FailingFs::default());
        // A one-byte cap rotates before every write after the first.
        let file = super::super::SizeRotatingFile::open_with(fs.clone(), path.clone(), 1, 2)
            .unwrap()
            .with_create_retry(Duration::ZERO);
        let mut sink = file.clone();
        let writer = EventWriter::spawn(move |bytes| sink.write_all(bytes), QUEUE_CAPACITY);

        writer.send(event(1));
        assert!(writer.flush(Duration::from_secs(5)));
        fs.fail_create.store(true, Ordering::SeqCst);
        for n in 2..=4 {
            writer.send(event(n));
            assert!(!writer.flush(Duration::from_secs(5)), "event {n}");
        }
        fs.fail_create.store(false, Ordering::SeqCst);
        writer.send(event(5));
        assert!(writer.flush(Duration::from_secs(5)));

        let current = std::fs::read_to_string(&path).unwrap();
        let note: AppEvent = serde_json::from_str(current.lines().next().unwrap()).unwrap();
        assert_eq!(note.summary, "Log events dropped");
        assert_eq!(note.data["dropped_events"], 3, "{current}");
        assert_eq!(numbers(current.as_bytes()), vec![5]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flush_waits_for_queued_events_and_reports_a_stuck_sink() {
        let written = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&written);
        let writer = EventWriter::spawn(
            move |bytes| {
                std::thread::sleep(Duration::from_millis(20));
                sink.lock().unwrap().extend_from_slice(bytes);
                Ok(())
            },
            QUEUE_CAPACITY,
        );
        for n in 0..5 {
            writer.send(event(n));
        }
        assert!(writer.flush(Duration::from_secs(5)));
        assert_eq!(numbers(&written.lock().unwrap()), vec![0, 1, 2, 3, 4]);

        // A sink that never returns makes flush time out instead of hanging.
        let stuck = EventWriter::spawn(
            |_| {
                std::thread::sleep(Duration::from_secs(30));
                Ok(())
            },
            4,
        );
        stuck.send(event(0));
        assert!(!stuck.flush(Duration::from_millis(100)));
    }
}
