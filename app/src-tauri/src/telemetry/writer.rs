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
    /// Acknowledged once every message queued before it has been written.
    Flush(SyncSender<()>),
}

#[derive(Clone)]
pub(super) struct EventWriter {
    queue: SyncSender<Message>,
    dropped: Arc<AtomicU64>,
    dropped_total: Arc<AtomicU64>,
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
                let mut acks: Vec<SyncSender<()>> = Vec::new();
                while let Ok(first) = receiver.recv() {
                    batch.clear();
                    acks.clear();
                    // Events dropped before these ones are noted first, so the
                    // file reads in the order things happened.
                    let lost = worker_dropped.swap(0, Ordering::Relaxed);
                    if lost > 0 {
                        batch.extend_from_slice(super::event_drop_note(lost).as_bytes());
                    }
                    let mut next = Some(first);
                    while let Some(message) = next.take() {
                        match message {
                            Message::Event(event) => {
                                if let Ok(line) = serde_json::to_string(&*event) {
                                    batch.extend_from_slice(line.as_bytes());
                                    batch.push(b'\n');
                                }
                            }
                            Message::Flush(ack) => acks.push(ack),
                        }
                        if batch.len() < MAX_BATCH_BYTES {
                            next = receiver.try_recv().ok();
                        }
                    }
                    if !batch.is_empty() {
                        let _ = sink(&batch);
                    }
                    for ack in acks.drain(..) {
                        let _ = ack.send(());
                    }
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

    /// Wait until everything queued so far is written, or `timeout` passes.
    /// Returns whether the queue drained.
    pub(crate) fn flush(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let (ack, acked) = mpsc::sync_channel(1);
        let mut message = Message::Flush(ack);
        loop {
            match self.queue.try_send(message) {
                Ok(()) => break,
                Err(TrySendError::Full(returned)) if Instant::now() < deadline => {
                    message = returned;
                    std::thread::sleep(Duration::from_millis(1));
                }
                Err(_) => return false,
            }
        }
        acked
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .is_ok()
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
