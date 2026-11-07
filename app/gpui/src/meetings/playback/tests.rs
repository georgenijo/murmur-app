use super::*;
use std::sync::{Mutex, atomic::AtomicUsize};
#[derive(Default)]
struct Output {
    generation: u64,
    queued: u64,
    played: u64,
    blocks: Vec<Vec<f32>>,
    started: bool,
}
#[derive(Default)]
struct FakeSink(Mutex<Output>);
impl Sink for FakeSink {
    fn reset(&self, g: u64) {
        let mut s = self.0.lock().unwrap();
        if g >= s.generation {
            *s = Output {
                generation: g,
                ..Default::default()
            };
        }
    }
    fn start(&self, g: u64) -> Result<(), Error> {
        let mut s = self.0.lock().unwrap();
        if g != s.generation {
            return Err(Error::Revoked);
        }
        s.started = true;
        Ok(())
    }
    fn enqueue(&self, g: u64, b: &[f32]) -> Result<(), Error> {
        let mut s = self.0.lock().unwrap();
        if g != s.generation || !s.started {
            return Err(Error::Revoked);
        }
        s.queued += b.len() as u64;
        s.blocks.push(b.to_vec());
        Ok(())
    }
    fn healthy(&self, g: u64) -> bool {
        let s = self.0.lock().unwrap();
        s.started && s.generation == g
    }
    fn queued_frames(&self) -> u64 {
        self.0.lock().unwrap().queued
    }
    fn played_frames(&self) -> u64 {
        self.0.lock().unwrap().played
    }
}
impl FakeSink {
    fn consume(&self) {
        let mut s = self.0.lock().unwrap();
        s.played += s.queued;
        s.queued = 0;
    }
}
struct AudioSource {
    reads: AtomicUsize,
    busy: std::sync::atomic::AtomicBool,
    revoke: Mutex<Option<Control>>,
    malformed: bool,
    oversized: bool,
}
impl Default for AudioSource {
    fn default() -> Self {
        Self {
            reads: AtomicUsize::new(0),
            busy: Default::default(),
            revoke: Default::default(),
            malformed: false,
            oversized: false,
        }
    }
}
fn wav(value: i16, duration: u64) -> Vec<u8> {
    let mut wav = crate::meetings::fixtures::wav(1);
    let count = (duration * 16) as usize;
    wav.truncate(44);
    wav[4..8].copy_from_slice(&((36 + 2 * count) as u32).to_le_bytes());
    wav[40..44].copy_from_slice(&((2 * count) as u32).to_le_bytes());
    for _ in 0..count {
        wav.extend(value.to_le_bytes());
    }
    wav
}
impl Source for AudioSource {
    fn admitted(&self, _: &str) -> Result<(), Error> {
        if self.busy.load(Ordering::SeqCst) {
            Err(Error::Busy)
        } else {
            Ok(())
        }
    }
    fn manifest(
        &self,
        _: &str,
        channel: Channel,
        from: u64,
        cursor: Option<Cursor>,
        _: usize,
    ) -> Result<Page, Error> {
        let chunks = if cursor.is_some() {
            vec![]
        } else {
            [0, 1000, 60000]
                .into_iter()
                .filter_map(|start| {
                    let end = start + 100;
                    (end > from).then_some(Chunk {
                        cursor: Cursor {
                            start_ms: start,
                            segment_id: if channel == Channel::Me { 1 } else { 2 },
                        },
                        channel,
                        end_ms: end,
                    })
                })
                .collect::<Vec<_>>()
        };
        let chunks = if self.oversized {
            vec![chunks[0].clone(); PAGE + 1]
        } else {
            chunks
        };
        Ok(Page {
            duration_ms: 60100,
            chunks,
            next: None,
        })
    }
    fn range(&self, _: &str, id: u64, offset: usize, length: usize) -> Result<Vec<u8>, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if let Some(c) = self.revoke.lock().unwrap().take() {
            c.pause();
        }
        let mut v = wav(if id == 1 { 4096 } else { 8192 }, 100);
        if self.malformed {
            v[0] = 0;
        }
        Ok(v[offset..v.len().min(offset + length)].to_vec())
    }
}
fn harness() -> (Player, Arc<AudioSource>, Arc<FakeSink>, Control) {
    let source = Arc::new(AudioSource::default());
    let sink = Arc::new(FakeSink::default());
    let control = Control::new(sink.clone());
    (
        Player::new(source.clone(), control.clone(), "synthetic-session".into()),
        source,
        sink,
        control,
    )
}
#[test]
fn overlaps_are_added_at_saved_levels_and_gaps_stay_silent() {
    let (mut p, source, sink, _) = harness();
    p.play(0, Channel::All).unwrap_or_else(|_| panic!());
    p.pump().unwrap_or_else(|_| panic!());
    assert!(
        sink.0.lock().unwrap().blocks[0]
            .iter()
            .all(|s| (*s - 0.375).abs() < 0.00001)
    );
    assert_eq!(source.reads.load(Ordering::SeqCst), 2);
    sink.consume();
    p.pump().unwrap_or_else(|_| panic!());
    assert!(sink.0.lock().unwrap().blocks[1].iter().all(|s| *s == 0.));
    assert_eq!(source.reads.load(Ordering::SeqCst), 2);
}
#[test]
fn seek_channel_and_long_gaps_do_not_eagerly_read_audio() {
    let (mut p, source, sink, _) = harness();
    p.play(40000, Channel::Them).unwrap_or_else(|_| panic!());
    p.pump().unwrap_or_else(|_| panic!());
    assert_eq!(source.reads.load(Ordering::SeqCst), 0);
    assert!(sink.0.lock().unwrap().blocks[0].iter().all(|s| *s == 0.));
    p.play(60000, Channel::Me).unwrap_or_else(|_| panic!());
    p.pump().unwrap_or_else(|_| panic!());
    assert_eq!(source.reads.load(Ordering::SeqCst), 1);
    assert!(
        sink.0.lock().unwrap().blocks[0]
            .iter()
            .all(|s| (*s - 0.125).abs() < 0.00001)
    );
}
#[test]
fn bounded_horizon_and_render_clock_position() {
    let (mut p, _, sink, _) = harness();
    p.play(1000, Channel::All).unwrap_or_else(|_| panic!());
    for _ in 0..10 {
        p.pump().unwrap_or_else(|_| panic!());
    }
    assert_eq!(sink.queued_frames(), 4800);
    assert_eq!(p.position_ms(), 1000);
    sink.consume();
    assert_eq!(p.position_ms(), 1300);
}
#[test]
fn pause_during_range_read_cannot_enqueue_or_restart() {
    let (mut p, source, sink, control) = harness();
    p.play(0, Channel::All).unwrap_or_else(|_| panic!());
    *source.revoke.lock().unwrap() = Some(control);
    assert!(matches!(p.pump(), Err(Error::Revoked)));
    assert_eq!(sink.queued_frames(), 0);
    assert!(!sink.0.lock().unwrap().started);
}
#[test]
fn busy_owner_revokes_queued_output_and_releases_caches() {
    let (mut p, source, sink, _) = harness();
    p.play(0, Channel::All).unwrap_or_else(|_| panic!());
    p.pump().unwrap_or_else(|_| panic!());
    source.busy.store(true, Ordering::SeqCst);
    assert!(matches!(p.pump(), Err(Error::Busy)));
    assert_eq!(sink.queued_frames(), 0);
    assert!(p.tracks.iter().all(|t| t.decoded.is_empty()));
}
#[test]
fn stale_start_enqueue_and_reset_cannot_replace_new_generation() {
    let sink = FakeSink::default();
    sink.reset(4);
    sink.reset(3);
    assert!(sink.start(3).is_err());
    assert!(sink.enqueue(3, &[0.]).is_err());
    assert_eq!(sink.0.lock().unwrap().generation, 4);
}
#[test]
fn invalid_wav_and_oversized_metadata_fail_closed() {
    for oversized in [false, true] {
        let source = Arc::new(AudioSource {
            malformed: !oversized,
            oversized,
            ..Default::default()
        });
        let sink = Arc::new(FakeSink::default());
        let c = Control::new(sink.clone());
        let mut p = Player::new(source, c, "fixture".into());
        p.play(0, Channel::All).unwrap_or_else(|_| panic!());
        assert!(matches!(p.pump(), Err(Error::Invalid)));
        assert_eq!(sink.queued_frames(), 0);
    }
}
#[test]
fn riff_parser_rejects_trailing_bytes_wrong_format_and_duration() {
    let v = wav(1, 100);
    assert!(decode(&v, 100).is_ok());
    assert!(decode(&v, 500).is_err());
    for offset in [0, 4, 8, 12, 16, 20, 22, 24, 28, 32, 34, 36, 40] {
        let mut v = v.clone();
        v[offset] ^= 0x7f;
        assert!(decode(&v, 100).is_err());
    }
    let mut v = v;
    v.push(0);
    assert!(decode(&v, 100).is_err());
}
#[test]
fn terminal_playback_stops_engine_and_keeps_end_position() {
    let (mut p, _, sink, _) = harness();
    p.play(60000, Channel::All).unwrap_or_else(|_| panic!());
    assert!(matches!(p.pump(), Ok(true)));
    sink.consume();
    assert!(matches!(p.pump(), Ok(false)));
    assert!(!sink.0.lock().unwrap().started);
    assert_eq!(p.position_ms(), 60100);
}
#[test]
fn pause_keeps_consumed_position_and_drops_decoded_audio() {
    let (mut p, _, sink, _) = harness();
    p.play(1000, Channel::Me).unwrap_or_else(|_| panic!());
    p.pump().unwrap_or_else(|_| panic!());
    sink.consume();
    p.pause();
    assert_eq!(p.position_ms(), 1100);
    assert!(p.tracks.iter().all(|t| t.decoded.is_empty()));
    assert_eq!(sink.queued_frames(), 0);
}

#[test]
fn stopped_output_is_detected_even_with_a_full_queue_and_can_restart() {
    let sink = Arc::new(FakeSink::default());
    let control = Control::new(sink.clone());
    let mut player = Player::new(Arc::new(AudioSource::default()), control, "fixture".into());
    assert!(player.play(0, Channel::All).is_ok());
    for _ in 0..3 {
        assert!(player.pump().unwrap_or_else(|_| panic!("pump failed")));
    }
    assert_eq!(sink.0.lock().unwrap().queued, 4800);
    sink.0.lock().unwrap().started = false;
    assert!(matches!(player.pump(), Err(Error::Output)));
    assert!(!player.pump().unwrap_or_else(|_| panic!("pump failed")));
    assert!(player.play(100, Channel::All).is_ok());
    assert!(player.pump().unwrap_or_else(|_| panic!("pump failed")));
}

#[test]
fn retired_worker_teardown_cannot_revoke_a_new_play_generation() {
    let sink = Arc::new(FakeSink::default());
    let control = Control::new(sink.clone());
    let retired = control.next();
    let current = control.next();
    assert!(sink.start(current).is_ok());
    control.pause_if_current(retired);
    assert!(control.current(current));
    assert!(sink.healthy(current));
    control.pause_if_current(current);
    assert!(!sink.healthy(current));
}
