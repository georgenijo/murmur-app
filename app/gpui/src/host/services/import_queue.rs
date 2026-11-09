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
    /// Admission into History has begun; a Cancel now can only be a request
    /// that the admission outcome decides.
    admitting: bool,
    cancel_requested: bool,
}
impl Item {
    /// Shipped `statusText`.
    pub fn status_text(&self) -> String {
        match self.status {
            Status::Queued => "Queued".into(),
            Status::Transcribing if self.cancel_requested => "Canceling…".into(),
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
                admitting: false,
                cancel_requested: false,
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
    /// The next queued item becomes active and gets its backend operation id in
    /// the same critical section, so a Cancel always has an operation to hit.
    /// With nothing left the runner role is released here too, so a concurrent
    /// `enqueue` either is seen or claims a fresh runner.
    fn advance(&mut self, new_operation: fn() -> String) -> Option<(u64, String, String)> {
        let Some(item) = self.items.iter_mut().find(|i| i.status == Status::Queued) else {
            self.running = false;
            return None;
        };
        item.status = Status::Transcribing;
        let operation = new_operation();
        item.operation = Some(operation.clone());
        let next = (item.id, item.path.clone(), operation);
        self.touch();
        Some(next)
    }
    /// The text is ready and admission into History is about to start. False
    /// when the item was canceled while transcribing, so nothing is admitted.
    fn begin_admission(&mut self, id: u64) -> bool {
        let Some(item) = self
            .item(id)
            .filter(|i| i.status == Status::Transcribing && !i.admitting)
        else {
            return false;
        };
        item.admitting = true;
        true
    }
    /// Record a failure of the active item. Returns false when the item was
    /// canceled meanwhile, so a late result never revives it.
    fn fail(&mut self, id: u64, message: String) -> bool {
        let Some(item) = self.item(id).filter(|i| i.status == Status::Transcribing) else {
            return false;
        };
        item.status = Status::Error;
        item.message = if item.cancel_requested {
            CANCELED.into()
        } else if message.is_empty() {
            "Transcription failed".into()
        } else {
            message
        };
        self.touch();
        true
    }
    /// Record the admission outcome. `published` means the transcript is in
    /// History, so it is Done even if Cancel was clicked first. Not published
    /// after a Cancel click means the cancellation won; not published without
    /// one means there was nothing to keep (History retention is off).
    fn settle(&mut self, id: u64, published: bool) -> bool {
        let Some(item) = self.item(id).filter(|i| i.status == Status::Transcribing) else {
            return false;
        };
        if published || !item.cancel_requested {
            item.status = Status::Done;
            item.done_at = Some(Instant::now());
        } else {
            item.status = Status::Error;
            item.message = CANCELED.into();
        }
        self.touch();
        true
    }
    /// Cancel a queued or active item. Returns the backend operation to
    /// cancel (when one was started), or `None` if there is nothing to cancel.
    /// Once admission has begun the toast stays `Transcribing` until the
    /// admission outcome decides between Done and Canceled.
    pub fn cancel(&mut self, id: u64) -> Option<Option<String>> {
        let item = self
            .item(id)
            .filter(|i| matches!(i.status, Status::Queued | Status::Transcribing))?;
        let operation = item.operation.clone();
        if item.admitting {
            if item.cancel_requested {
                return None;
            }
            item.cancel_requested = true;
        } else {
            item.status = Status::Error;
            item.message = CANCELED.into();
        }
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
/// shared `transcribe_file` command, the native `Services::accept_file` and
/// `Services::cancel_file`.
pub trait Engine: Send + Sync {
    fn transcribe(&self, path: String, operation: String) -> Fut<'_, Result<(), String>>;
    /// Publish the staged transcript. `Ok(true)` means it is in History;
    /// `Ok(false)` means nothing was published (a cancellation won, or there
    /// was nothing to retain). An `Err` leaves the staged text pending.
    fn admit(&self, operation: String) -> Fut<'_, Result<bool, String>>;
    /// Record cancellation intent and release the staged text. Synchronous and
    /// never waits, so a click is ordered before any admission that has not
    /// started yet, and a failed admission cannot leave content pending.
    fn cancel(&self, operation: &str);
}
fn lock(queue: &Mutex<Queue>) -> std::sync::MutexGuard<'_, Queue> {
    queue.lock().unwrap_or_else(|p| p.into_inner())
}

/// Run queued items sequentially until none remain. The caller must hold the
/// runner role from `Queue::claim`. Every failure is recorded on its own item
/// and releases that item's staged transcript.
pub async fn drain(queue: &Mutex<Queue>, engine: &dyn Engine, new_operation: fn() -> String) {
    loop {
        let next = lock(queue).advance(new_operation);
        let Some((id, path, operation)) = next else {
            return;
        };
        if let Err(error) = engine.transcribe(path, operation.clone()).await {
            lock(queue).fail(id, error);
            continue;
        }
        if !lock(queue).begin_admission(id) {
            // Canceled after the text was produced: discard it instead of
            // publishing to History.
            engine.cancel(&operation);
            continue;
        }
        match engine.admit(operation.clone()).await {
            Ok(published) => {
                lock(queue).settle(id, published);
            }
            Err(error) => {
                // The failed admission keeps its staged transcript for retry.
                // Nothing retries it, so release it now; otherwise repeated
                // failures fill the pending-file capacity and block imports.
                engine.cancel(&operation);
                lock(queue).fail(id, error);
            }
        }
    }
}

/// Cancel from a toast: record the request in the queue, then tell the backend
/// synchronously (outside the queue lock). Admission that already won still
/// shows Done; one that has not started loses to the recorded intent.
pub fn cancel_import(queue: &Mutex<Queue>, engine: &dyn Engine, id: u64) {
    let cancelled = lock(queue).cancel(id);
    if let Some(Some(operation)) = cancelled {
        engine.cancel(&operation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use murmur_core::completion::{Command, CompletionService, producers::Producers};
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
    fn name_of(path: &str) -> &str {
        path.rsplit('/').next().unwrap_or(path)
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
                let name = name_of(&path).to_owned();
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
        fn admit(&self, operation: String) -> Fut<'_, Result<bool, String>> {
            Box::pin(async move {
                let name = self.path_of(&operation);
                self.log(format!("accept {name}"));
                if self.fail_accept.contains(&name.as_str()) {
                    return Err("Storage unavailable".into());
                }
                Ok(true)
            })
        }
        fn cancel(&self, operation: &str) {
            let name = self.path_of(operation);
            self.log(format!("cancel {name}"));
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
        // Only the failed admission releases staged content; nothing else cancels.
        assert_eq!(count(&fake, "cancel"), 1);
        assert_eq!(count(&fake, "cancel d.wav"), 1);
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
        assert!(!q.settle(b, true));
        assert!(!q.fail(b, "late".into()));
        assert_eq!(q.visible()[1].status, Status::Error);
        assert_eq!(q.visible()[1].message, CANCELED);
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
        assert!(q.advance(operation).is_some());
        assert!(q.advance(operation).is_some());
        assert!(
            q.advance(operation).is_none(),
            "queue empty: runner role released"
        );
        assert!(q.claim());
    }

    // The tests below drive the drain loop and the Cancel path against the real
    // completion `Producers` (the owner behind `Services::accept_file` and
    // `Services::cancel_file`), not the fake: the fake's failed admission stores
    // no pending content and its cancel only fires during transcription.

    struct Root(std::path::PathBuf);
    impl Root {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "murmur-import-queue-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::SeqCst)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    type Hook = Box<dyn FnOnce(&Real) + Send>;
    struct Real {
        queue: Arc<Mutex<Queue>>,
        producers: Producers,
        service: CompletionService,
        /// Stage a transcript into History (false: retainHistory is off).
        retain: bool,
        during_transcribe: Mutex<Option<Hook>>,
        before_accept: Mutex<Option<Hook>>,
        after_accept: Mutex<Option<Hook>>,
        // Dropped last, after the service releases its root.
        root: Root,
    }
    impl Real {
        fn new(retain: bool) -> Self {
            let root = Root::new();
            let service = CompletionService::start(root.0.clone(), false).unwrap();
            service.execute(Command::Snapshot).unwrap();
            Self {
                queue: Arc::default(),
                producers: Producers::new(service.clone(), "synthetic".into()),
                service,
                retain,
                during_transcribe: Mutex::default(),
                before_accept: Mutex::default(),
                after_accept: Mutex::default(),
                root,
            }
        }
        fn history(&self) -> usize {
            self.service
                .execute(Command::Snapshot)
                .unwrap()
                .history
                .len()
        }
        fn run(&self, names: &[&str]) {
            lock(&self.queue).enqueue(&paths(names));
            assert!(lock(&self.queue).claim());
            futures::executor::block_on(drain(&self.queue, self, operation));
        }
        /// Click Cancel on a file's toast, exactly like the window does.
        fn click(&self, name: &str) {
            let id = lock(&self.queue)
                .items
                .iter()
                .find(|i| i.name == name)
                .map(|i| i.id)
                .unwrap();
            cancel_import(&self.queue, self, id);
        }
        fn toast(&self, name: &str) -> (Status, String) {
            let queue = lock(&self.queue);
            let item = queue.items.iter().find(|i| i.name == name).unwrap();
            (item.status, item.status_text())
        }
        fn fire(hook: &Mutex<Option<Hook>>, real: &Real) {
            let hook = hook.lock().unwrap().take();
            if let Some(hook) = hook {
                hook(real);
            }
        }
    }
    impl Engine for Real {
        fn transcribe(&self, path: String, operation: String) -> Fut<'_, Result<(), String>> {
            Box::pin(async move {
                self.producers
                    .begin_file(&operation)
                    .map_err(|e| e.to_string())?;
                Self::fire(&self.during_transcribe, self);
                let name = name_of(&path);
                self.producers.finish_file(
                    &operation,
                    Some(serde_json::json!({"schemaVersion":2,"pinned":false,"source":"file","id":name,"text":"synthetic file","timestamp":1,"duration":2})),
                    self.retain,
                );
                Ok(())
            })
        }
        fn admit(&self, operation: String) -> Fut<'_, Result<bool, String>> {
            Box::pin(async move {
                Self::fire(&self.before_accept, self);
                let published = self
                    .producers
                    .accept_file(&operation)
                    .map(|snapshot| snapshot.is_some())
                    .map_err(|e| e.to_string());
                Self::fire(&self.after_accept, self);
                published
            })
        }
        fn cancel(&self, operation: &str) {
            self.producers.cancel_file(operation);
        }
    }

    #[test]
    fn failed_admissions_release_their_staged_text_so_imports_recover() {
        let real = Real::new(true);
        // A directory at the holding file makes every admission fail with a
        // storage error, which the real producers keep for retry.
        let holding = real.root.0.join("completion-holding.json");
        let saved = std::fs::read(&holding).unwrap();
        std::fs::remove_file(&holding).unwrap();
        std::fs::create_dir(&holding).unwrap();
        // More failures than the pending-file capacity (200).
        let names: Vec<String> = (0..210).map(|i| format!("f{i}.wav")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        real.run(&names);
        for (name, status, message) in snapshot(&real.queue) {
            assert_eq!(status, Status::Error, "{name}");
            assert!(
                !message.contains("Too many"),
                "{name} hit the pending cap: {message}"
            );
        }
        // Storage recovers. The History view's next snapshot settles the owner's
        // own queue; no released failure may resurrect into History, and the
        // next import must not be blocked by the failures.
        std::fs::remove_dir(&holding).unwrap();
        std::fs::write(&holding, saved).unwrap();
        assert_eq!(real.history(), 0, "failed imports never reach History");
        real.run(&["after.wav"]);
        assert_eq!(
            real.toast("after.wav"),
            (Status::Done, "Done".to_owned()),
            "imports recover after storage does"
        );
        assert_eq!(real.history(), 1);
    }

    #[test]
    fn cancel_before_admission_starts_wins_and_nothing_reaches_history() {
        let real = Real::new(true);
        *real.before_accept.lock().unwrap() = Some(Box::new(|real| {
            real.click("a.wav");
            // The click is a request until admission settles; it is neither
            // success nor a premature "Canceled".
            assert_eq!(
                real.toast("a.wav"),
                (Status::Transcribing, "Canceling…".to_owned())
            );
            real.click("a.wav"); // a second click changes nothing
        }));
        real.run(&["a.wav", "b.wav"]);
        assert_eq!(real.toast("a.wav"), (Status::Error, CANCELED.to_owned()));
        assert_eq!(real.toast("b.wav").0, Status::Done, "the batch continues");
        assert_eq!(real.history(), 1, "only b.wav is in History");
    }

    #[test]
    fn admission_that_already_won_shows_done_even_if_cancel_arrives_after() {
        let real = Real::new(true);
        *real.after_accept.lock().unwrap() = Some(Box::new(|real| real.click("a.wav")));
        real.run(&["a.wav"]);
        assert_eq!(real.toast("a.wav"), (Status::Done, "Done".to_owned()));
        assert_eq!(real.history(), 1, "the published transcript stays");
    }

    #[test]
    fn cancel_while_transcribing_discards_the_text_and_never_admits() {
        let real = Real::new(true);
        *real.during_transcribe.lock().unwrap() = Some(Box::new(|real| real.click("a.wav")));
        real.run(&["a.wav", "b.wav"]);
        assert_eq!(real.toast("a.wav"), (Status::Error, CANCELED.to_owned()));
        assert_eq!(real.toast("b.wav").0, Status::Done);
        assert_eq!(real.history(), 1);
    }

    #[test]
    fn nothing_to_retain_is_done_not_canceled() {
        let real = Real::new(false);
        real.run(&["a.wav"]);
        assert_eq!(real.toast("a.wav"), (Status::Done, "Done".to_owned()));
        assert_eq!(real.history(), 0);
    }
}
