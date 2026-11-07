//! Synthetic-only host: no app database, settings files, Calendar, capture or clipboard.
use super::{host::*, inbox::Inbox, model::*, playback};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};
pub fn meeting(id: &str) -> Meeting {
    Meeting {
        id: id.into(),
        title: Some("Synthetic planning meeting".into()),
        title_source: Some("manual".into()),
        attendees: vec!["Fixture participant".into()],
        started_at_ms: 1700000000000,
        duration_ms: 65000,
        status: "complete".into(),
        retain_audio: true,
        segment_count: 4,
        preview: "Synthetic meeting transcript for native review.".into(),
        search_matches: vec![],
    }
}
pub fn snapshot() -> Snapshot {
    let session = meeting("fixture-1");
    Snapshot {
        revision: 1,
        phase: Phase::Idle,
        recording_generation: 0,
        recording_session: None,
        elapsed_ms: 0,
        microphone_active: false,
        system_audio_active: false,
        capture_busy: false,
        permission: Permission::Granted,
        needs_relaunch: false,
        failure: None,
        echo_notice: None,
        meetings: vec![
            session.clone(),
            Meeting {
                title: None,
                id: "fixture-2".into(),
                retain_audio: false,
                ..session.clone()
            },
        ],
        query: String::new(),
        total: 2,
        detail: Some(Detail {
            session,
            segments: (0..4)
                .map(|i| Segment {
                    id: i + 1,
                    session_id: "fixture-1".into(),
                    channel: if i % 2 == 0 {
                        Channel::Me
                    } else {
                        Channel::Them
                    },
                    remote_speaker: if i % 2 == 0 { None } else { Some(1) },
                    start_ms: i * 1000,
                    end_ms: i * 1000 + 1500,
                    status: if i == 2 {
                        "pending"
                    } else if i == 3 {
                        "failed"
                    } else {
                        "final"
                    }
                    .into(),
                    text: "Synthetic transcript. All audio and names in this app are fixtures."
                        .into(),
                    audio_available: i < 2,
                })
                .collect(),
            me: "Me".into(),
            them: "Them".into(),
            remote: vec![RemoteSpeaker {
                id: 1,
                label: "Speaker 1".into(),
            }],
            review_revision: None,
        }),
        calendar: Calendar::Closed,
        bundle: None,
        audio: Audio {
            state: AudioState::Ready,
            duration_ms: 65000,
            ..Default::default()
        },
        notice: None,
    }
}
pub struct FixtureHost {
    pub state: RefCell<Snapshot>,
    pub actions: RefCell<Vec<Action>>,
    pub calls: RefCell<Vec<&'static str>>,
    pub stops: Rc<Cell<usize>>,
    pub pauses: Cell<usize>,
    pub fail: Cell<bool>,
    pub native_audio: Cell<bool>,
    pub held: RefCell<Vec<(Request, Inbox)>>,
    pub hold: Cell<bool>,
    player: RefCell<Option<playback::Player>>,
    control: RefCell<Option<playback::Control>>,
}
impl Default for FixtureHost {
    fn default() -> Self {
        Self {
            state: RefCell::new(snapshot()),
            actions: Default::default(),
            calls: Default::default(),
            stops: Default::default(),
            pauses: Cell::new(0),
            fail: Cell::new(false),
            native_audio: Cell::new(false),
            held: Default::default(),
            hold: Cell::new(false),
            player: Default::default(),
            control: Default::default(),
        }
    }
}
struct Stop(Rc<Cell<usize>>);
impl Subscription for Stop {
    fn stop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}
impl Host for FixtureHost {
    fn subscribe(
        &self,
        c: murmur_core::events::Consumer,
        _: u64,
        _: Inbox,
    ) -> Box<dyn Subscription> {
        assert_eq!(c, murmur_core::events::Consumer::Main);
        self.calls.borrow_mut().push("subscribe");
        Box::new(Stop(self.stops.clone()))
    }
    fn pause_playback(&self) {
        self.pauses.set(self.pauses.get() + 1);
        let position = self
            .player
            .borrow()
            .as_ref()
            .map(playback::Player::position_ms);
        if let Some(p) = self.player.borrow_mut().as_mut() {
            p.pause();
        } else if let Some(c) = self.control.borrow().as_ref() {
            c.pause();
        }
        let mut state = self.state.borrow_mut();
        if state.audio.state == AudioState::Playing {
            state.audio.state = AudioState::Paused;
            if let Some(p) = position {
                state.audio.position_ms = p.min(state.audio.duration_ms);
            }
        }
    }
    fn discard_calendar(&self) {
        self.state.borrow_mut().calendar = Calendar::Closed;
    }
    fn request(&self, r: Request, inbox: Inbox) {
        if !r.lease.is_current() || inbox.is_closed() {
            return;
        }
        self.calls.borrow_mut().push("request");
        self.actions.borrow_mut().push(r.action.clone());
        if self.hold.get() {
            self.held.borrow_mut().push((r, inbox));
            return;
        }
        let result = (|| {
            if self.fail.replace(false) {
                return Err(Problem::Unavailable);
            }
            let mut s = self.state.borrow().clone();
            if !matches!(r.action, Action::Read) && r.ticket.revision != s.revision {
                return Err(Problem::Conflict);
            }
            s.notice = None;
            match r.action {
                Action::Start | Action::StartSuggested(_) if !s.busy() => {
                    self.pause_playback();
                    s.phase = Phase::Recording;
                    s.recording_generation = s.revision + 1;
                    s.recording_session = Some("fixture-capture".into());
                    s.microphone_active = true;
                    s.system_audio_active = true;
                }
                Action::Stop { generation }
                    if s.phase.active() && s.recording_generation == generation =>
                {
                    s.phase = Phase::Processing;
                    s.microphone_active = false;
                    s.system_audio_active = false;
                }
                Action::Start | Action::StartSuggested(_) | Action::Stop { .. } => {
                    return Err(Problem::Conflict);
                }
                Action::Select(id) => {
                    s.detail = snapshot()
                        .detail
                        .filter(|d| d.session.id == id)
                        .or_else(|| {
                            s.meetings.iter().find(|m| m.id == id).map(|m| Detail {
                                session: m.clone(),
                                segments: vec![],
                                me: "Me".into(),
                                them: "Them".into(),
                                remote: vec![],
                                review_revision: None,
                            })
                        });
                    s.audio = Audio {
                        state: if s.detail.as_ref().is_some_and(|d| d.session.retain_audio) {
                            AudioState::Ready
                        } else {
                            AudioState::NotRetained
                        },
                        duration_ms: 65000,
                        ..Default::default()
                    };
                    s.calendar = Calendar::Closed;
                    s.bundle = None;
                }
                Action::List { query, .. } => {
                    s.meetings = snapshot()
                        .meetings
                        .into_iter()
                        .filter(|m| {
                            query.is_empty()
                                || m.title.as_ref().is_some_and(|t| {
                                    t.to_lowercase().contains(&query.to_lowercase())
                                })
                                || m.preview.to_lowercase().contains(&query.to_lowercase())
                        })
                        .collect();
                    s.total = s.meetings.len();
                    s.query = query;
                    if s.detail
                        .as_ref()
                        .is_some_and(|d| !s.meetings.iter().any(|m| m.id == d.session.id))
                    {
                        s.detail = None;
                    }
                }
                Action::SaveMetadata {
                    id,
                    title,
                    attendees,
                } => {
                    let d = s
                        .detail
                        .as_mut()
                        .filter(|d| d.session.id == id)
                        .ok_or(Problem::Conflict)?;
                    d.session.title = title;
                    d.session.attendees = attendees;
                    d.session.title_source = Some("manual".into());
                    s.notice = Some("Meeting details saved on this Mac.".into());
                }
                Action::SaveLabels {
                    id,
                    revision,
                    me,
                    them,
                } => {
                    let d = s
                        .detail
                        .as_mut()
                        .filter(|d| d.session.id == id && d.review_revision == revision)
                        .ok_or(Problem::Conflict)?;
                    d.me = me;
                    d.them = them;
                    d.review_revision = Some(revision.unwrap_or(0) + 1);
                    s.notice = Some("Speaker labels saved on this Mac.".into());
                }
                Action::RenameSpeaker { id, speaker, label } => {
                    let d = s
                        .detail
                        .as_mut()
                        .filter(|d| d.session.id == id)
                        .ok_or(Problem::Conflict)?;
                    d.remote
                        .iter_mut()
                        .find(|r| r.id == speaker)
                        .ok_or(Problem::Invalid)?
                        .label = label;
                    s.notice = Some("Speaker name saved for this meeting.".into());
                }
                Action::CalendarLookup(_) => {
                    s.calendar = Calendar::Choosing(vec![Candidate {
                        token: "synthetic-selection-token".into(),
                        title: "Synthetic calendar event".into(),
                        attendees: vec!["Fixture guest".into()],
                        start_ms: 1700000000000,
                        end_ms: 1700000065000,
                    }])
                }
                Action::CalendarApply { id, token } => {
                    let candidate = match &s.calendar {
                        Calendar::Choosing(c) => c
                            .iter()
                            .find(|c| c.token == token)
                            .ok_or(Problem::Conflict)?,
                        _ => return Err(Problem::Conflict),
                    };
                    let d = s
                        .detail
                        .as_mut()
                        .filter(|d| d.session.id == id)
                        .ok_or(Problem::Conflict)?;
                    d.session.title = Some(candidate.title.clone());
                    d.session.attendees = candidate.attendees.clone();
                    d.session.title_source = Some("calendar".into());
                    s.calendar = Calendar::Closed;
                    s.notice = Some("Meeting named from calendar.".into());
                }
                Action::CalendarReset => {
                    s.calendar = Calendar::Closed;
                    s.notice = Some(
                        "Calendar access reset. Choose Name from calendar to request access again."
                            .into(),
                    );
                }
                Action::BundlePreview(_) => {
                    s.bundle = Some(Bundle {
                        audio_segments: 2,
                        audio_bytes: 96088,
                        unavailable: 1,
                        max_bytes: 1024 * 1024 * 1024,
                    })
                }
                Action::BundleExport { .. } => {
                    s.bundle = None;
                    s.notice =
                        Some("Synthetic bundle export completed; no file was written.".into());
                }
                Action::Copy { .. } => {
                    s.notice =
                        Some("Synthetic copy completed; the pasteboard was not accessed.".into())
                }
                Action::Export { .. } => {
                    s.notice = Some("Synthetic export completed; no file was written.".into())
                }
                Action::Delete(id) => {
                    s.meetings.retain(|m| m.id != id);
                    s.total = s.meetings.len();
                    s.detail = None;
                    s.notice = Some("Meeting deleted from this Mac.".into());
                }
                Action::DeleteAll => {
                    s.meetings.clear();
                    s.total = 0;
                    s.detail = None;
                    s.notice = Some("All meeting transcripts deleted from this Mac.".into());
                }
                Action::CheckAccess => {
                    s.permission = Permission::Granted;
                    s.notice = Some("System Audio access is ready.".into());
                }
                Action::Playback { action, .. } => {
                    let position = match action {
                        PlaybackAction::Seek(ms) => ms,
                        PlaybackAction::Segment { start_ms, .. } => start_ms,
                        _ => s.audio.position_ms,
                    };
                    let channel = match action {
                        PlaybackAction::Channel(c) | PlaybackAction::Segment { channel: c, .. } => {
                            c
                        }
                        _ => s.audio.channel,
                    };
                    let playing = matches!(
                        action,
                        PlaybackAction::Play | PlaybackAction::Segment { .. }
                    ) || s.audio.state == AudioState::Playing;
                    s.audio.position_ms = position.min(s.audio.duration_ms);
                    s.audio.channel = channel;
                    if matches!(action, PlaybackAction::Pause) {
                        self.pause_playback();
                        s.audio.position_ms = self.state.borrow().audio.position_ms;
                        s.audio.state = AudioState::Paused;
                    } else if playing {
                        #[cfg(target_os = "macos")]
                        if self.native_audio.get() {
                            if self.control.borrow().is_none() {
                                let output = Arc::new(
                                    playback::NativeSink::new()
                                        .map_err(|_| Problem::Unavailable)?,
                                );
                                let control = playback::Control::new(output);
                                *self.player.borrow_mut() = Some(playback::Player::new(
                                    Arc::new(SyntheticAudio),
                                    control.clone(),
                                    "fixture-1".into(),
                                ));
                                *self.control.borrow_mut() = Some(control);
                            }
                            let mut player = self.player.borrow_mut();
                            let p = player.as_mut().unwrap();
                            p.play(position, channel)
                                .map_err(|_| Problem::Unavailable)?;
                            for _ in 0..3 {
                                p.pump().map_err(|_| Problem::Unavailable)?;
                            }
                        }
                        s.audio.state = AudioState::Playing;
                    } else {
                        s.audio.state = AudioState::Ready;
                    }
                }
                _ => (),
            }
            if let Some(d) = &s.detail
                && let Some(row) = s.meetings.iter_mut().find(|m| m.id == d.session.id)
            {
                *row = d.session.clone();
            }
            s.revision += 1;
            *self.state.borrow_mut() = s.clone();
            Ok(s)
        })();
        let _ = inbox.publish(Event::Reply {
            ticket: r.ticket,
            result,
        });
    }
    fn tick(&self, attachment: u64, inbox: Inbox) {
        let mut player = self.player.borrow_mut();
        let Some(p) = player.as_mut() else {
            return;
        };
        if self.state.borrow().audio.state != AudioState::Playing {
            return;
        }
        let result = p.pump();
        let mut s = self.state.borrow_mut();
        s.audio.position_ms = p.position_ms().min(s.audio.duration_ms);
        if !matches!(result, Ok(true)) {
            s.audio.state = if result.is_err() {
                AudioState::Error
            } else {
                AudioState::Ready
            };
        }
        s.revision += 1;
        let _ = inbox.publish(Event::Changed {
            attachment,
            snapshot: s.clone(),
        });
    }
}
/// Three short PCM16 tones on a 65-second two-channel timeline, never real audio.
pub struct SyntheticAudio;
impl playback::Source for SyntheticAudio {
    fn admitted(&self, _: &str) -> Result<(), playback::Error> {
        Ok(())
    }
    fn manifest(
        &self,
        _: &str,
        channel: Channel,
        from: u64,
        cursor: Option<playback::Cursor>,
        _: usize,
    ) -> Result<playback::Page, playback::Error> {
        let chunks = if cursor.is_none() {
            (0..2)
                .filter_map(|i| {
                    let start = if channel == Channel::Me { 0 } else { 1000 } + i * 62000;
                    let end = (start + 1500).min(65000);
                    (end > from && start < 65000).then_some(playback::Chunk {
                        cursor: playback::Cursor {
                            start_ms: start,
                            segment_id: if channel == Channel::Me { 1 } else { 2 },
                        },
                        channel,
                        end_ms: end,
                    })
                })
                .collect()
        } else {
            vec![]
        };
        Ok(playback::Page {
            duration_ms: 65000,
            chunks,
            next: None,
        })
    }
    fn range(
        &self,
        _: &str,
        id: u64,
        offset: usize,
        length: usize,
    ) -> Result<Vec<u8>, playback::Error> {
        let wav = wav(id);
        if offset > wav.len() {
            return Err(playback::Error::Invalid);
        }
        Ok(wav[offset..wav.len().min(offset + length)].to_vec())
    }
}
pub fn wav(id: u64) -> Vec<u8> {
    let samples = 24000u32;
    let mut v = Vec::new();
    v.extend(b"RIFF");
    v.extend((36 + samples * 2).to_le_bytes());
    v.extend(b"WAVEfmt ");
    v.extend(16u32.to_le_bytes());
    v.extend(1u16.to_le_bytes());
    v.extend(1u16.to_le_bytes());
    v.extend(16000u32.to_le_bytes());
    v.extend(32000u32.to_le_bytes());
    v.extend(2u16.to_le_bytes());
    v.extend(16u16.to_le_bytes());
    v.extend(b"data");
    v.extend((samples * 2).to_le_bytes());
    for i in 0..samples {
        let sample = ((i as f32 * std::f32::consts::TAU * (if id == 1 { 220. } else { 330. })
            / 16000.)
            .sin()
            * 700.) as i16;
        v.extend(sample.to_le_bytes());
    }
    v
}
impl super::Meetings {
    pub fn fixture_state(
        &mut self,
        state: &str,
        w: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let Some(s) = self.model.snapshot.as_mut() else {
            return;
        };
        match state {
            "empty"=>{s.meetings.clear();s.total=0;s.detail=None;},
            "search"=>{s.meetings.clear();s.total=0;s.detail=None;s.query="unmatched fixture".into();},
            "starting"|"recording"|"stopping"|"processing"=>{s.phase=match state{"starting"=>Phase::Starting,"recording"=>Phase::Recording,"stopping"=>Phase::Stopping,_=>Phase::Processing};s.recording_generation=42;s.recording_session=Some("fixture-capture".into());s.elapsed_ms=65000;s.microphone_active=state=="recording";s.system_audio_active=state=="recording";},
            "denied"=>{s.permission=Permission::Denied;s.failure=Some(error_message("system_audio_permission_denied"));},
            "unsupported"=>s.permission=Permission::Unsupported,
            "relaunch"=>{s.permission=Permission::Unknown;s.needs_relaunch=true;},
            "echo"=>s.echo_notice=Some("Speaker echo reduction is unavailable. Murmur is keeping the original microphone audio for the rest of this meeting.".into()),
            "calendar"=>s.calendar=Calendar::Choosing(vec![Candidate{token:"fixture-token".into(),title:"Synthetic calendar event".into(),attendees:vec!["Fixture guest".into()],start_ms:1700000000000,end_ms:1700000065000}]),
            "calendar-empty"=>s.calendar=Calendar::Choosing(vec![]),
            "calendar-denied"=>s.calendar=Calendar::Unavailable{message:"Calendar access is not allowed. Enable it in System Settings, or name this meeting manually.",denied:true,unsupported:false},
            "bundle"=>s.bundle=Some(Bundle{audio_segments:2,audio_bytes:96088,unavailable:1,max_bytes:1024*1024*1024}),
            "not-retained"=>{s.detail.as_mut().unwrap().session.retain_audio=false;s.audio.state=AudioState::NotRetained;},
            "audio-error"=>s.audio.state=AudioState::Error,
            "audio-busy"=>{s.capture_busy=true;s.audio.state=AudioState::Paused;},
            "playing"=>{s.audio.state=AudioState::Playing;s.audio.position_ms=1000;},
            "loading"=>{self.model.snapshot=None;self.model.pending=true;return;},
            "error"=>self.model.problem=Some(Problem::Unavailable),_=>(),
        }
        if matches!(state, "naming" | "text-input") {
            self.name(w, cx);
        }
        if state == "speakers" {
            self.speakers = true;
            self.ensure_fields(w, cx);
        }
        cx.notify();
    }
    /// Internal editor hook: same focus/input handler as native text delivery;
    /// no system HID, shared pasteboard, focus activation or synthetic AX typing.
    pub fn fixture_type(&mut self, w: &mut gpui::Window, cx: &mut gpui::Context<Self>) {
        use gpui::{EntityInputHandler, Focusable};
        self.name(w, cx);
        let input = self.fields["title"].clone();
        w.focus(&input.read(cx).focus_handle(cx), cx);
        input.update(cx, |s, cx| {
            s.replace_text_in_range(None, "Synthetic typed meeting", w, cx)
        });
    }
}
