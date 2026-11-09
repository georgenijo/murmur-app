//! Ordered file-import queue shared by the picker, command palette and native
//! drop. Imports run one at a time (the transcription backend is shared with
//! live dictation); one file's failure is recorded on its own item and never
//! ends the batch. The queue holds file names and status only, no transcript.
//! A queued import is a background job: it is not tied to the visible section,
//! so navigating or hiding the window does not cancel it (shipped behavior).
use std::{
    future::Future,
    path::Path,
    pin::Pin,
    sync::Mutex,
    time::{Duration, Instant},
};

/// Shipped wording (`UNSUPPORTED_MESSAGE` in `app/src/lib/fileQueue.ts`).
pub const UNSUPPORTED: &str = "Unsupported file type. Use WAV, MP3, M4A, MP4, or MOV.";
pub const CANCELED: &str = "Canceled";
/// The bundled decoder handles exactly these containers (docs/features/history-workspace.md).
pub const EXTENSIONS: [&str; 5] = ["wav", "mp3", "m4a", "mp4", "mov"];
/// A finished item stays visible this long, like the shipped toast.
pub const DONE_LINGER: Duration = Duration::from_secs(3);
/// Only the newest items are shown, like the shipped toast stack.
pub const VISIBLE: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Queued,
    Transcribing,
    Done,
    Error,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub id: u64,
    pub path: String,
    pub name: String,
    pub status: Status,
    /// Failure reason once `Error`.
    pub message: String,
    operation: Option<String>,
    done_at: Option<Instant>,
}
impl Item {
    /// Shipped `statusText`.
    pub fn status_text(&self) -> String {
        match self.status {
            Status::Queued => "Queued".into(),
            Status::Transcribing => "Transcribing…".into(),
            Status::Done => "Done".into(),
            Status::Error if self.message.is_empty() => "Could not transcribe".into(),
            Status::Error => self.message.clone(),
        }
    }
}

pub fn supported(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

#[derive(Default)]
pub struct Queue {
    next: u64,
    items: Vec<Item>,
    running: bool,
    /// Whole-selection refusal (nothing transcribable), shown above the items.
    pub error: Option<&'static str>,
    /// Bumped on every visible change; the window polls it.
    pub revision: u64,
}
impl Queue {
    fn touch(&mut self) {
        self.revision = self.revision.wrapping_add(1);
    }
    fn item(&mut self, id: u64) -> Option<&mut Item> {
        self.items.iter_mut().find(|i| i.id == id)
    }
    /// Queue every supported path that is not already listed. A mixed
    /// selection queues its valid files; the unsupported error is shown only
    /// when none qualify. There is no count cap. Returns how many were added.
    pub fn enqueue(&mut self, paths: &[String]) -> usize {
        if paths.is_empty() {
            return 0;
        }
        if !paths.iter().any(|p| supported(p)) {
            self.error = Some(UNSUPPORTED);
            self.touch();
            return 0;
        }
        self.error = None;
        let mut added = 0;
        for path in paths {
            if !supported(path) || self.items.iter().any(|i| &i.path == path) {
                continue;
            }
            self.next += 1;
            let name = Path::new(path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(path)
                .to_owned();
            self.items.push(Item {
                id: self.next,
                path: path.clone(),
                name,
                status: Status::Queued,
                message: String::new(),
                operation: None,
                done_at: None,
            });
            added += 1;
        }
        self.touch();
        added
    }
    /// True for the one caller that must start the drain loop.
    pub fn claim(&mut self) -> bool {
        !std::mem::replace(&mut self.running, true)
    }
    /// The next queued item becomes active. With nothing left the runner role
    /// is released in the same critical section, so a concurrent `enqueue`
    /// either is seen here or claims a fresh runner.
    fn advance(&mut self) -> Option<(u64, String)> {
        let Some(item) = self.items.iter_mut().find(|i| i.status == Status::Queued) else {
            self.running = false;
            return None;
        };
        item.status = Status::Transcribing;
        let next = (item.id, item.path.clone());
        self.touch();
        Some(next)
    }
    fn set_operation(&mut self, id: u64, operation: &str) {
        if let Some(item) = self.item(id) {
            item.operation = Some(operation.to_owned());
        }
    }
    /// Record the outcome of the active item. Returns false when the item was
    /// canceled meanwhile, so a late result never revives it.
    fn finish(&mut self, id: u64, result: Result<(), String>) -> bool {
        let Some(item) = self.item(id).filter(|i| i.status == Status::Transcribing) else {
            return false;
        };
        match result {
            Ok(()) => {
                item.status = Status::Done;
                item.done_at = Some(Instant::now());
            }
            Err(message) => {
                item.status = Status::Error;
                item.message = if message.is_empty() {
                    "Transcription failed".into()
                } else {
                    message
                };
            }
        }
        self.touch();
        true
    }
    /// Cancel a queued or active item. Returns the backend operation to
    /// cancel (when one was started), or `None` if the item is not cancelable.
    pub fn cancel(&mut self, id: u64) -> Option<Option<String>> {
        let item = self
            .item(id)
            .filter(|i| matches!(i.status, Status::Queued | Status::Transcribing))?;
        item.status = Status::Error;
        item.message = CANCELED.into();
        let operation = item.operation.clone();
        self.touch();
        Some(operation)
    }
    pub fn dismiss(&mut self, id: u64) {
        let before = self.items.len();
        self.items
            .retain(|i| i.id != id || !matches!(i.status, Status::Done | Status::Error));
        if self.items.len() != before {
            self.touch();
        }
    }
    pub fn dismiss_error(&mut self) {
        if self.error.take().is_some() {
            self.touch();
        }
    }
    /// Drop finished items after `DONE_LINGER`. Errors stay until dismissed.
    pub fn expire(&mut self, now: Instant) {
        let before = self.items.len();
        self.items.retain(|i| {
            i.status != Status::Done
                || i.done_at
                    .is_none_or(|at| now.duration_since(at) < DONE_LINGER)
        });
        if self.items.len() != before {
            self.touch();
        }
    }
    /// The newest `VISIBLE` items, newest first.
    pub fn visible(&self) -> Vec<Item> {
        self.items.iter().rev().take(VISIBLE).cloned().collect()
    }
}

pub type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
/// The service boundary the drain loop drives: production maps these onto the
/// shared `transcribe_file`, `accept_file_transcription` and
/// `cancel_file_transcription` commands.
pub trait Engine: Send + Sync {
    fn transcribe(&self, path: String, operation: String) -> Fut<'_, Result<(), String>>;
    fn accept(&self, operation: String) -> Fut<'_, Result<(), String>>;
    fn cancel(&self, operation: String) -> Fut<'_, ()>;
}
fn lock(queue: &Mutex<Queue>) -> std::sync::MutexGuard<'_, Queue> {
    queue.lock().unwrap_or_else(|p| p.into_inner())
}

/// Run queued items sequentially until none remain. The caller must hold the
/// runner role from `Queue::claim`. Every failure is recorded on its own item.
pub async fn drain(queue: &Mutex<Queue>, engine: &dyn Engine, new_operation: fn() -> String) {
    loop {
        let next = lock(queue).advance();
        let Some((id, path)) = next else {
            return;
        };
        let operation = new_operation();
        lock(queue).set_operation(id, &operation);
        let result = engine.transcribe(path, operation.clone()).await;
        let result = match result {
            Ok(()) => {
                let active = lock(queue)
                    .item(id)
                    .is_some_and(|i| i.status == Status::Transcribing);
                if active {
                    engine.accept(operation.clone()).await
                } else {
                    // Canceled after the text was produced: discard it
                    // instead of publishing to History.
                    engine.cancel(operation).await;
                    continue;
                }
            }
            Err(error) => Err(error),
        };
        if !lock(queue).finish(id, result) {
            engine.cancel(operation).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    };

    fn operation() -> String {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        format!("op-{}", NEXT.fetch_add(1, Ordering::SeqCst))
    }
    fn paths(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| format!("/synthetic/{n}")).collect()
    }
    fn snapshot(queue: &Mutex<Queue>) -> Vec<(String, Status, String)> {
        lock(queue)
            .items
            .iter()
            .map(|i| (i.name.clone(), i.status, i.message.clone()))
            .collect()
    }
    /// A fake service boundary. Failures and cancel hooks are keyed by file name.
    #[derive(Default)]
    struct Fake {
        queue: Arc<Mutex<Queue>>,
        fail_transcribe: Vec<&'static str>,
        fail_accept: Vec<&'static str>,
        /// Cancel this file's toast while its transcription is in flight.
        cancel_while_running: Vec<&'static str>,
        log: Mutex<Vec<String>>,
    }
    impl Fake {
        fn name(path: &str) -> &str {
            path.rsplit('/').next().unwrap_or(path)
        }
        fn log(&self, line: String) {
            self.log.lock().unwrap().push(line);
        }
        fn path_of(&self, operation: &str) -> String {
            self.log
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find_map(|l| l.strip_suffix(&format!(" {operation}")))
                .and_then(|l| l.strip_prefix("transcribe "))
                .unwrap_or("")
                .to_owned()
        }
    }
    impl Engine for Fake {
        fn transcribe(&self, path: String, operation: String) -> Fut<'_, Result<(), String>> {
            Box::pin(async move {
                let name = Self::name(&path).to_owned();
                self.log(format!("transcribe {name} {operation}"));
                if self.cancel_while_running.contains(&name.as_str()) {
                    let id = lock(&self.queue)
                        .items
                        .iter()
                        .find(|i| i.name == name)
                        .map(|i| i.id)
                        .unwrap();
                    assert!(lock(&self.queue).cancel(id).is_some());
                }
                if self.fail_transcribe.contains(&name.as_str()) {
                    return Err("No supported audio track found.".into());
                }
                Ok(())
            })
        }
        fn accept(&self, operation: String) -> Fut<'_, Result<(), String>> {
            Box::pin(async move {
                let name = self.path_of(&operation);
                self.log(format!("accept {name}"));
                if self.fail_accept.contains(&name.as_str()) {
                    return Err("Storage unavailable".into());
                }
                Ok(())
            })
        }
        fn cancel(&self, operation: String) -> Fut<'_, ()> {
            Box::pin(async move {
                let name = self.path_of(&operation);
                self.log(format!("cancel {name}"));
            })
        }
    }
    fn run(fake: &Fake) {
        assert!(lock(&fake.queue).claim());
        futures::executor::block_on(drain(&fake.queue, fake, operation));
    }
    fn count(fake: &Fake, prefix: &str) -> usize {
        fake.log
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.starts_with(prefix))
            .count()
    }

    #[test]
    fn one_failed_file_does_not_abort_the_batch_and_names_its_reason() {
        let fake = Fake {
            fail_transcribe: vec!["b.mp4"],
            fail_accept: vec!["d.wav"],
            ..Default::default()
        };
        lock(&fake.queue).enqueue(&paths(&["a.wav", "b.mp4", "c.mp3", "d.wav", "e.mov"]));
        run(&fake);
        assert_eq!(
            snapshot(&fake.queue),
            vec![
                ("a.wav".into(), Status::Done, String::new()),
                (
                    "b.mp4".into(),
                    Status::Error,
                    "No supported audio track found.".into()
                ),
                ("c.mp3".into(), Status::Done, String::new()),
                ("d.wav".into(), Status::Error, "Storage unavailable".into()),
                ("e.mov".into(), Status::Done, String::new()),
            ]
        );
        // The failed transcription is never accepted into History.
        assert_eq!(count(&fake, "accept"), 4);
        assert_eq!(count(&fake, "accept b.mp4"), 0);
        // The runner role is released only after the last item.
        assert!(lock(&fake.queue).claim());
    }

    #[test]
    fn large_batches_are_not_capped() {
        let fake = Fake::default();
        let names: Vec<String> = (0..150).map(|i| format!("/synthetic/f{i}.wav")).collect();
        assert_eq!(lock(&fake.queue).enqueue(&names), 150);
        run(&fake);
        assert_eq!(count(&fake, "accept"), 150);
        assert!(snapshot(&fake.queue).iter().all(|i| i.1 == Status::Done));
    }

    #[test]
    fn unsupported_only_selection_reports_and_a_mixed_one_queues_valid_files() {
        let mut q = Queue::default();
        assert_eq!(q.enqueue(&paths(&["notes.txt", "clip.flac"])), 0);
        assert_eq!(q.error, Some(UNSUPPORTED));
        assert!(q.visible().is_empty());
        assert_eq!(q.enqueue(&paths(&["notes.txt", "clip.MP3", "clip.MP3"])), 1);
        assert_eq!(q.error, None, "a valid file clears the refusal");
        assert_eq!(q.visible()[0].name, "clip.MP3");
        assert_eq!(q.enqueue(&paths(&["clip.MP3"])), 0, "already listed");
        q.error = Some(UNSUPPORTED);
        q.dismiss_error();
        assert_eq!(q.error, None);
    }

    #[test]
    fn canceling_the_active_file_discards_it_and_the_next_one_still_runs() {
        let fake = Fake {
            cancel_while_running: vec!["a.wav"],
            ..Default::default()
        };
        lock(&fake.queue).enqueue(&paths(&["a.wav", "b.wav"]));
        run(&fake);
        assert_eq!(
            snapshot(&fake.queue),
            vec![
                ("a.wav".into(), Status::Error, CANCELED.into()),
                ("b.wav".into(), Status::Done, String::new()),
            ]
        );
        assert_eq!(
            count(&fake, "accept a.wav"),
            0,
            "canceled text is never published"
        );
        assert_eq!(count(&fake, "cancel a.wav"), 1);
        assert_eq!(count(&fake, "accept b.wav"), 1);
    }

    #[test]
    fn canceling_a_queued_file_skips_it_and_a_late_result_cannot_revive_it() {
        let fake = Fake::default();
        lock(&fake.queue).enqueue(&paths(&["a.wav", "b.wav", "c.wav"]));
        let b = lock(&fake.queue).visible()[1].id;
        assert_eq!(lock(&fake.queue).cancel(b), Some(None));
        assert_eq!(
            lock(&fake.queue).cancel(b),
            None,
            "only pending work cancels"
        );
        run(&fake);
        assert_eq!(count(&fake, "transcribe b.wav"), 0);
        assert_eq!(count(&fake, "accept"), 2);
        let mut q = lock(&fake.queue);
        assert!(!q.finish(b, Ok(())));
        assert_eq!(q.visible()[1].status, Status::Error);
    }

    #[test]
    fn done_items_expire_but_errors_wait_for_dismissal_and_only_three_show() {
        let fake = Fake {
            fail_transcribe: vec!["b.wav"],
            ..Default::default()
        };
        lock(&fake.queue).enqueue(&paths(&["a.wav", "b.wav", "c.wav", "d.wav"]));
        run(&fake);
        let mut q = lock(&fake.queue);
        let names: Vec<_> = q.visible().into_iter().map(|i| i.name).collect();
        assert_eq!(
            names,
            ["d.wav", "c.wav", "b.wav"],
            "newest first, three shown"
        );
        let revision = q.revision;
        q.expire(Instant::now());
        assert_eq!(q.revision, revision, "fresh Done items stay visible");
        q.expire(Instant::now() + DONE_LINGER);
        assert_eq!(q.items.len(), 1);
        let error = q.visible()[0].clone();
        assert_eq!(
            (error.name.as_str(), error.status),
            ("b.wav", Status::Error)
        );
        assert_eq!(error.status_text(), "No supported audio track found.");
        q.dismiss(error.id);
        assert!(q.items.is_empty());
    }

    #[test]
    fn a_batch_added_while_running_is_picked_up_without_a_second_runner() {
        let mut q = Queue::default();
        q.enqueue(&paths(&["a.wav"]));
        assert!(q.claim());
        q.enqueue(&paths(&["b.wav"]));
        assert!(!q.claim(), "the active runner owns the new item");
        assert!(q.advance().is_some());
        assert!(q.advance().is_some());
        assert!(q.advance().is_none(), "queue empty: runner role released");
        assert!(q.claim());
    }
}
