//! Exercise the GUI's native_audio branch without opening an audio device.
//! Like NativeSink, this seam refuses start/enqueue on the request thread and
//! fences reset independently of a stalled start. Only synthetic PCM is read.
use super::*;
use crate::meetings::{
    host::Host,
    playback::{Error, Sink},
};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

struct Output {
    request_thread: std::thread::ThreadId,
    generation: AtomicU64,
    running: AtomicU64,
    played: AtomicU64,
    starts: AtomicUsize,
    finished: AtomicUsize,
    blocks: AtomicUsize,
    fail: AtomicBool,
    stalled: Mutex<bool>,
    release: Condvar,
}
impl Output {
    fn new(stalled: bool, fail: bool) -> Arc<Self> {
        Arc::new(Self {
            request_thread: std::thread::current().id(),
            generation: AtomicU64::new(0),
            running: AtomicU64::new(0),
            played: AtomicU64::new(0),
            starts: AtomicUsize::new(0),
            finished: AtomicUsize::new(0),
            blocks: AtomicUsize::new(0),
            fail: AtomicBool::new(fail),
            stalled: Mutex::new(stalled),
            release: Condvar::new(),
        })
    }
    fn release_start(&self) {
        *self.stalled.lock().unwrap() = false;
        self.release.notify_all();
    }
}
impl Sink for Output {
    fn reset(&self, generation: u64) {
        if generation > self.generation.fetch_max(generation, Ordering::SeqCst) {
            self.running.store(0, Ordering::SeqCst);
            self.played.store(0, Ordering::SeqCst);
        }
    }
    fn start(&self, generation: u64) -> Result<(), Error> {
        if std::thread::current().id() == self.request_thread {
            return Err(Error::Output);
        }
        self.starts.fetch_add(1, Ordering::SeqCst);
        let stalled = self.stalled.lock().unwrap();
        let _guard = self
            .release
            .wait_while(stalled, |stalled| *stalled)
            .unwrap();
        let result = if self.fail.load(Ordering::SeqCst) {
            Err(Error::Output)
        } else if generation != self.generation.load(Ordering::SeqCst) {
            Err(Error::Revoked)
        } else {
            self.running.store(generation, Ordering::SeqCst);
            Ok(())
        };
        self.finished.fetch_add(1, Ordering::SeqCst);
        result
    }
    fn enqueue(&self, generation: u64, samples: &[f32]) -> Result<(), Error> {
        if std::thread::current().id() == self.request_thread {
            return Err(Error::Output);
        }
        if !self.healthy(generation) {
            return Err(Error::Revoked);
        }
        self.blocks.fetch_add(1, Ordering::SeqCst);
        self.played
            .fetch_add(samples.len() as u64, Ordering::SeqCst);
        Ok(())
    }
    fn healthy(&self, generation: u64) -> bool {
        generation == self.generation.load(Ordering::SeqCst)
            && generation == self.running.load(Ordering::SeqCst)
    }
    fn queued_frames(&self) -> u64 {
        0
    }
    fn played_frames(&self) -> u64 {
        self.played.load(Ordering::SeqCst)
    }
}
fn wait_for(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !predicate() {
        assert!(Instant::now() < deadline, "playback worker did not finish");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn play(e: &Entity<Meetings>, cx: &mut VisualTestContext) {
    e.update(cx, |s, cx| {
        s.request(
            Action::Playback {
                id: "fixture-1".into(),
                action: PlaybackAction::Play,
            },
            cx,
        )
    });
    draw(cx);
}
fn tick(e: &Entity<Meetings>, h: &FixtureHost, cx: &mut VisualTestContext) {
    e.update(cx, |s, cx| {
        h.tick(s.model.attachment, s.inbox.clone());
        s.drain(cx);
    });
}
fn native_host(
    cx: &mut TestAppContext,
    stalled: bool,
    fail: bool,
) -> (
    Entity<Meetings>,
    Rc<FixtureHost>,
    Arc<Output>,
    &mut VisualTestContext,
) {
    let (e, h, cx) = harness(cx);
    let output = Output::new(stalled, fail);
    h.install_audio_sink(output.clone());
    h.native_audio.set(true);
    (e, h, output, cx)
}
#[gpui::test]
fn gui_request_starts_and_pumps_native_audio_on_worker(cx: &mut TestAppContext) {
    let (e, h, output, cx) = native_host(cx, false, false);
    play(&e, cx);
    assert!(e.read_with(cx, |s, _| s.model.pending
        && s.model.snapshot.as_ref().unwrap().audio.state == AudioState::Buffering));
    wait_for(|| {
        tick(&e, &h, cx);
        e.read_with(cx, |s, _| !s.model.pending)
    });
    assert!(e.read_with(cx, |s, _| s.model.problem.is_none()
        && s.model.snapshot.as_ref().unwrap().audio.state == AudioState::Playing));
    assert!(output.blocks.load(Ordering::SeqCst) > 0);
    h.pause_playback();
}
#[gpui::test]
fn gui_start_failure_becomes_async_problem(cx: &mut TestAppContext) {
    let (e, h, _, cx) = native_host(cx, false, true);
    play(&e, cx);
    wait_for(|| {
        tick(&e, &h, cx);
        e.read_with(cx, |s, _| !s.model.pending)
    });
    assert!(
        e.read_with(cx, |s, _| s.model.problem == Some(Problem::Unavailable)
            && s.model.snapshot.as_ref().unwrap().audio.state
                == AudioState::Error)
    );
}
#[gpui::test]
fn gui_pause_during_stalled_start_returns_before_release_and_discards_late_start(
    cx: &mut TestAppContext,
) {
    let (e, h, output, cx) = native_host(cx, true, false);
    play(&e, cx);
    wait_for(|| output.starts.load(Ordering::SeqCst) == 1);
    let before = Instant::now();
    e.update(cx, |s, cx| {
        s.request(
            Action::Playback {
                id: "fixture-1".into(),
                action: PlaybackAction::Pause,
            },
            cx,
        )
    });
    assert!(before.elapsed() < Duration::from_millis(200));
    draw(cx);
    assert!(e.read_with(cx, |s, _| !s.model.pending
        && s.model.snapshot.as_ref().unwrap().audio.state == AudioState::Paused));
    output.release_start();
    wait_for(|| output.finished.load(Ordering::SeqCst) == 1);
    tick(&e, &h, cx);
    assert_eq!(output.blocks.load(Ordering::SeqCst), 0);
    assert!(e.read_with(cx, |s, _| s.model.problem.is_none()
        && s.model.snapshot.as_ref().unwrap().audio.state == AudioState::Paused));
    // A fresh Play still succeeds; the retired attempt cannot reset its output.
    play(&e, cx);
    wait_for(|| {
        tick(&e, &h, cx);
        output.blocks.load(Ordering::SeqCst) > 0
    });
    assert!(e.read_with(cx, |s, _| s.model.problem.is_none()
        && s.model.snapshot.as_ref().unwrap().audio.state == AudioState::Playing));
    h.pause_playback();
}
#[gpui::test]
fn hide_discards_stalled_start_failure_and_reopen_keeps_new_state(cx: &mut TestAppContext) {
    let (e, h, output, cx) = native_host(cx, true, true);
    play(&e, cx);
    wait_for(|| output.starts.load(Ordering::SeqCst) == 1);
    cx.update(|w, cx| e.update(cx, |s, cx| s.hide(w, cx)));
    output.release_start();
    wait_for(|| output.finished.load(Ordering::SeqCst) == 1);
    e.update(cx, |s, cx| s.attach(cx));
    draw(cx);
    tick(&e, &h, cx);
    assert!(e.read_with(cx, |s, _| s.model.problem.is_none()
        && s.model.snapshot.as_ref().unwrap().audio.state == AudioState::Paused));
    assert_eq!(output.blocks.load(Ordering::SeqCst), 0);
}
#[test]
fn self_test_entry_uses_worker_with_native_thread_contract() {
    let output = Output::new(false, false);
    assert!(fixtures::audio_self_test(output.clone()).is_ok());
    assert_eq!(output.starts.load(Ordering::SeqCst), 3);
    assert!(output.blocks.load(Ordering::SeqCst) >= 3);
    assert_eq!(output.running.load(Ordering::SeqCst), 0);
}

#[gpui::test]
fn navigation_during_stalled_start_is_immediate_and_late_result_cannot_change_selection(
    cx: &mut TestAppContext,
) {
    let (e, h, output, cx) = native_host(cx, true, false);
    play(&e, cx);
    wait_for(|| output.starts.load(Ordering::SeqCst) == 1);
    let before = Instant::now();
    e.update(cx, |s, cx| s.request(Action::Back, cx));
    assert!(before.elapsed() < Duration::from_millis(200));
    draw(cx);
    assert!(matches!(h.actions.borrow().last(), Some(Action::Back)));
    assert!(e.read_with(cx, |s, _| !s.model.pending));
    e.update(cx, |s, cx| s.select("fixture-2".into(), cx));
    draw(cx);
    output.release_start();
    wait_for(|| output.finished.load(Ordering::SeqCst) == 1);
    tick(&e, &h, cx);
    assert!(e.read_with(cx, |s, _| {
        s.model.problem.is_none()
            && s.model
                .snapshot
                .as_ref()
                .unwrap()
                .detail
                .as_ref()
                .unwrap()
                .session
                .id
                == "fixture-2"
            && s.model.snapshot.as_ref().unwrap().audio.state == AudioState::NotRetained
    }));
    assert_eq!(output.blocks.load(Ordering::SeqCst), 0);
}
