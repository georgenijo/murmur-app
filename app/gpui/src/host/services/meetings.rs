//! Meeting presentation and playback adapt the shared recording/store owners.
use super::{
    backend::Backend,
    calendar::{apply_outcome, lookup_calendar},
    os::Live,
};
use crate::{
    meetings::{host::*, model::*, playback as p},
    workspace::{Event, Problem},
};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
#[derive(Clone, Default)]
struct Selection {
    query: String,
    limit: usize,
    id: Option<String>,
    calendar: Calendar,
    bundle: Option<Bundle>,
    audio: Audio,
    notice: Option<String>,
}
#[derive(Default)]
pub struct Presentation {
    selection: Mutex<Selection>,
    epoch: AtomicU64,
    player: Mutex<Option<(String, p::Worker)>>,
    closed: AtomicBool,
    retired: Mutex<Vec<p::Worker>>,
    revision: AtomicU64,
    reads: tokio::sync::Mutex<()>,
    last_tick: Mutex<Option<std::time::Instant>>,
    refreshing: Arc<AtomicBool>,
}
impl Presentation {
    pub fn quiesce(&self, deadline: std::time::Instant) -> bool {
        self.closed.store(true, Ordering::Release);
        let player = self.player.lock().unwrap_or_else(|p| p.into_inner()).take();
        let mut workers =
            std::mem::take(&mut *self.retired.lock().unwrap_or_else(|p| p.into_inner()));
        if let Some((_, worker)) = player {
            workers.push(worker);
        }
        let mut done = true;
        for worker in &mut workers {
            done &= worker.drain(deadline);
        }
        done
    }
}
fn strv(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or_default().into()
}
fn n(v: &Value, k: &str) -> u64 {
    v[k].as_u64().unwrap_or(0)
}
fn channel(v: &Value) -> Channel {
    match v.as_str() {
        Some("me") => Channel::Me,
        Some("them") => Channel::Them,
        _ => Channel::All,
    }
}
fn channel_wire(c: Channel) -> &'static str {
    match c {
        Channel::Me => "me",
        Channel::Them => "them",
        Channel::All => "all",
    }
}
fn meeting(v: &Value) -> Result<Meeting, String> {
    Ok(Meeting {
        id: v["id"].as_str().ok_or("Invalid meeting")?.into(),
        title: v["title"].as_str().map(String::from),
        title_source: v["titleSource"].as_str().map(String::from),
        attendees: v["attendees"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(String::from)
            .collect(),
        started_at_ms: n(v, "startedAtMs") as i64,
        duration_ms: n(v, "durationMs"),
        status: strv(v, "status"),
        retain_audio: v["retainAudio"] == true,
        segment_count: n(v, "segmentCount") as usize,
        preview: strv(v, "preview"),
        search_matches: vec![],
    })
}
fn detail(v: &Value) -> Result<Detail, String> {
    let session = meeting(&v["session"])?;
    let segments = v["segments"]
        .as_array()
        .ok_or("Invalid meeting detail")?
        .iter()
        .map(|s| Segment {
            id: n(s, "id"),
            session_id: strv(s, "sessionId"),
            channel: channel(&s["speaker"]),
            remote_speaker: s["remoteSpeakerId"].as_u64().map(|n| n as u32),
            start_ms: n(s, "startMs"),
            end_ms: n(s, "endMs"),
            status: strv(s, "status"),
            text: strv(s, "text"),
            audio_available: s["audioAvailable"] == true,
        })
        .collect();
    Ok(Detail {
        session,
        segments,
        me: v["labels"]["me"].as_str().unwrap_or("Me").into(),
        them: v["labels"]["them"].as_str().unwrap_or("Them").into(),
        remote: v["remoteSpeakers"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| RemoteSpeaker {
                id: n(r, "speakerId") as u32,
                label: strv(r, "label"),
            })
            .collect(),
        review_revision: v["review"]["revision"].as_u64(),
    })
}
pub fn read(b: Backend) -> super::backend::ReadFuture<Snapshot> {
    Box::pin(async move {
        let _read = b.meetings.reads.lock().await;
        let revision = b.meetings.revision.fetch_add(1, Ordering::SeqCst) + 1;
        let epoch = b.meetings.epoch.load(Ordering::Acquire);
        let mut selection = b
            .meetings
            .selection
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        let status = b.call("get_meeting_status", json!({})).await?;
        let permission = b
            .call("get_system_audio_permission_status", json!({}))
            .await?;
        let busy = b.call("get_meeting_audio_capture_busy", json!({})).await? == true;
        let limit = selection.limit.clamp(50, 10000);
        let mut rows = Vec::new();
        let mut total;
        loop {
            let page=b.call("list_meetings",json!({"query":if selection.query.is_empty(){None}else{Some(&selection.query)},"offset":rows.len(),"limit":(limit-rows.len()).min(100)})).await?;
            total = page["total"].as_u64().ok_or("Meeting list unavailable")? as usize;
            let chunk = page["sessions"]
                .as_array()
                .ok_or("Meeting list unavailable")?;
            for row in chunk {
                let mut m = meeting(row)?;
                m.search_matches = page["searchMatches"][&m.id]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(String::from)
                    .collect();
                rows.push(m);
            }
            if chunk.is_empty() || rows.len() >= total || rows.len() >= limit {
                break;
            }
        }
        let selected = selection.id.as_deref().or_else(|| {
            matches!(
                status["phase"].as_str(),
                Some("starting" | "recording" | "stopping" | "processing")
            )
            .then(|| status["sessionId"].as_str())
            .flatten()
        });
        let detail = if let Some(id) = selected {
            Some(detail(&b.call("get_meeting", json!({"id":id})).await?)?)
        } else {
            None
        };
        if let Some(detail) = &detail {
            let manifest = b.call("get_meeting_audio_manifest", json!({"sessionId":detail.session.id,"fromMs":0,"channel":"all","cursor":null,"limit":1})).await?;
            selection.audio.duration_ms = n(&manifest, "durationMs");
            if !matches!(
                selection.audio.state,
                AudioState::Playing | AudioState::Paused | AudioState::Buffering
            ) {
                selection.audio.state = match manifest["kind"].as_str() {
                    Some("available") => AudioState::Ready,
                    _ => match manifest["reason"].as_str() {
                        Some("notRetained") => AudioState::NotRetained,
                        Some("notFinished") => AudioState::NotFinished,
                        Some("noAudio") => AudioState::NoAudio,
                        _ => AudioState::Error,
                    },
                };
            }
        }
        if b.meetings.epoch.load(Ordering::Acquire) != epoch {
            return Err("Meeting selection changed".into());
        }
        let phase = match status["phase"].as_str() {
            Some("starting") => Phase::Starting,
            Some("recording") => Phase::Recording,
            Some("stopping") => Phase::Stopping,
            Some("processing") => Phase::Processing,
            Some("failed") => Phase::Failed,
            _ => Phase::Idle,
        };
        if busy {
            b.pause_player();
        }
        Ok(Snapshot {
            revision,
            phase,
            recording_generation: n(&status, "generation"),
            recording_session: status["sessionId"].as_str().map(String::from),
            elapsed_ms: n(&status, "elapsedMs"),
            microphone_active: status["microphoneActive"] == true,
            system_audio_active: status["systemAudioActive"] == true,
            capture_busy: busy,
            permission: match permission.as_str() {
                Some("granted") => Permission::Granted,
                Some("denied") => Permission::Denied,
                Some("unsupported") => Permission::Unsupported,
                _ => Permission::Unknown,
            },
            needs_relaunch: false,
            failure: None,
            echo_notice: None,
            meetings: rows,
            query: selection.query,
            total,
            detail,
            calendar: selection.calendar,
            bundle: selection.bundle,
            audio: selection.audio,
            notice: selection.notice,
        })
    })
}
impl Backend {
    fn pause_player(&self) {
        if let Some((_, worker)) = self
            .meetings
            .player
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
        {
            worker.pause();
        }
    }
    fn retire_player(&self) {
        let player = self
            .meetings
            .player
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take();
        if let Some((_, mut worker)) = player {
            worker.terminate();
            let mut retired = self
                .meetings
                .retired
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            retired.retain(|worker| !worker.finished());
            if !worker.finished() {
                retired.push(worker);
            }
        }
    }
    fn player_action(&self, id: String, action: PlaybackAction) -> Result<(), String> {
        let mut player = self
            .meetings
            .player
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if self.meetings.closed.load(Ordering::Acquire) {
            return Err("Playback is closing".into());
        }
        {
            let mut retired = self
                .meetings
                .retired
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            retired.retain(|worker| !worker.finished());
            if !retired.is_empty() {
                return Err("Playback teardown is still finishing".into());
            }
        }
        let mut selection = self
            .meetings
            .selection
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if player.as_ref().is_none_or(|(current, _)| current != &id) {
            let sink = Arc::new(p::NativeSink::new().map_err(|_| "Audio output unavailable")?);
            *player = Some((
                id.clone(),
                p::Worker::new(
                    Arc::new(AudioSource(self.clone())),
                    p::Control::new(sink),
                    id,
                ),
            ));
        }
        let worker = &mut player.as_mut().ok_or("Audio output unavailable")?.1;
        match action {
            PlaybackAction::Pause => {
                worker.pause();
                selection.audio.state = AudioState::Paused;
            }
            PlaybackAction::Channel(channel) => {
                selection.audio.channel = channel;
                worker
                    .play(worker.position_ms(), channel)
                    .map_err(|_| "Audio playback unavailable")?;
                selection.audio.state = AudioState::Buffering;
            }
            PlaybackAction::Seek(position) => {
                worker
                    .play(position, selection.audio.channel)
                    .map_err(|_| "Audio playback unavailable")?;
                selection.audio.state = AudioState::Buffering;
            }
            PlaybackAction::Segment { channel, start_ms } => {
                selection.audio.channel = channel;
                worker
                    .play(start_ms, channel)
                    .map_err(|_| "Audio playback unavailable")?;
                selection.audio.state = AudioState::Buffering;
            }
            PlaybackAction::Play | PlaybackAction::Retry => {
                worker
                    .play(worker.position_ms(), selection.audio.channel)
                    .map_err(|_| "Audio playback unavailable")?;
                selection.audio.state = AudioState::Buffering;
            }
        }
        Ok(())
    }
}
struct AudioSource(Backend);
impl p::Source for AudioSource {
    fn admitted(&self, session: &str) -> Result<(), p::Error> {
        let busy = self
            .0
            .runtime
            .block_on(self.0.call("get_meeting_audio_capture_busy", json!({})))
            .map_err(|_| p::Error::Unavailable)?;
        if busy == true {
            return Err(p::Error::Unavailable);
        }
        let result = self
            .0
            .runtime
            .block_on(self.0.call(
                "get_meeting_audio_manifest",
                json!({"sessionId":session,"fromMs":0,"channel":"all","cursor":null,"limit":1}),
            ))
            .map_err(|_| p::Error::Unavailable)?;
        if result["kind"] != "available" {
            return Err(p::Error::Unavailable);
        }
        Ok(())
    }
    fn manifest(
        &self,
        session: &str,
        c: Channel,
        from_ms: u64,
        cursor: Option<p::Cursor>,
        limit: usize,
    ) -> Result<p::Page, p::Error> {
        let result=self.0.runtime.block_on(self.0.call("get_meeting_audio_manifest",json!({"sessionId":session,"fromMs":from_ms,"channel":channel_wire(c),"cursor":cursor.map(|c|json!({"startMs":c.start_ms,"segmentId":c.segment_id})),"limit":limit}))).map_err(|_|p::Error::Unavailable)?;
        if result["kind"] != "available" {
            return Err(p::Error::Unavailable);
        }
        let chunks = result["chunks"]
            .as_array()
            .ok_or(p::Error::Invalid)?
            .iter()
            .map(|v| p::Chunk {
                cursor: p::Cursor {
                    start_ms: n(v, "startMs"),
                    segment_id: n(v, "segmentId"),
                },
                channel: channel(&v["channel"]),
                end_ms: n(v, "endMs"),
            })
            .collect();
        Ok(p::Page {
            duration_ms: n(&result, "durationMs"),
            chunks,
            next: result
                .get("nextCursor")
                .filter(|v| v.is_object())
                .map(|v| p::Cursor {
                    start_ms: n(v, "startMs"),
                    segment_id: n(v, "segmentId"),
                }),
        })
    }
    fn range(
        &self,
        session: &str,
        segment: u64,
        offset: usize,
        length: usize,
    ) -> Result<Vec<u8>, p::Error> {
        self.0
            .runtime
            .block_on(self.0.services.audio_range(
                session.into(),
                segment as i64,
                offset as u64,
                length as u32,
            ))
            .map_err(|_| p::Error::Unavailable)
    }
}
impl Host for Backend {
    fn subscribe(
        &self,
        c: murmur_core::events::Consumer,
        a: u64,
        inbox: crate::meetings::inbox::Inbox,
    ) -> Box<dyn Subscription> {
        Box::new(self.watch(c, read, move |snapshot| {
            let _ = inbox.publish(Event::Changed {
                attachment: a,
                snapshot,
            });
        }))
    }
    fn pause_playback(&self) {
        self.pause_player();
    }
    fn discard_calendar(&self) {
        self.meetings
            .selection
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .calendar = Calendar::Closed;
    }
    fn tick(&self, a: u64, inbox: crate::meetings::inbox::Inbox) {
        if self.activity.busy() {
            return;
        }
        let update = self
            .meetings
            .player
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_mut()
            .and_then(|(_, w)| w.poll());
        let due = {
            let mut last = self
                .meetings
                .last_tick
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if last.is_none_or(|last| last.elapsed() >= std::time::Duration::from_secs(1)) {
                *last = Some(std::time::Instant::now());
                true
            } else {
                false
            }
        };
        let changed = update.is_some();
        if let Some(update) = update {
            {
                let mut selection = self
                    .meetings
                    .selection
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                match update {
                    Ok(p) => {
                        selection.audio.position_ms = p.position_ms;
                        selection.audio.state = if p.playing {
                            AudioState::Playing
                        } else {
                            AudioState::Paused
                        };
                    }
                    Err(_) => selection.audio.state = AudioState::Error,
                }
            }
        }
        if (changed || due)
            && self
                .meetings
                .refreshing
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            let b = self.clone();
            self.spawn(async move {
                struct Release(Arc<AtomicBool>);
                impl Drop for Release {
                    fn drop(&mut self) {
                        self.0.store(false, Ordering::Release);
                    }
                }
                let _release = Release(b.meetings.refreshing.clone());
                if let Ok(snapshot) = read(b.clone()).await
                    && !b.activity.busy()
                {
                    let _ = inbox.publish(Event::Changed {
                        attachment: a,
                        snapshot,
                    });
                }
            });
        }
    }
    fn request(&self, r: Request, inbox: crate::meetings::inbox::Inbox) {
        if !r.lease.is_current() || inbox.is_closed() {
            return;
        }
        // A confirmation belongs to the action that produced it; the next action clears it.
        self.meetings
            .selection
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .notice = None;
        // Selection is accepted synchronously, before a preceding read can finish.
        match &r.action {
            Action::Select(id) => {
                self.retire_player();
                self.meetings
                    .selection
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .audio = Audio::default();
                self.meetings
                    .selection
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .id = Some(id.clone());
                self.meetings.epoch.fetch_add(1, Ordering::AcqRel);
            }
            Action::List { query, limit } => {
                let mut selection = self
                    .meetings
                    .selection
                    .lock()
                    .unwrap_or_else(|p| p.into_inner());
                selection.query = query.clone();
                selection.limit = *limit;
                self.meetings.epoch.fetch_add(1, Ordering::AcqRel);
            }
            _ => {}
        }
        let b = self.clone();
        let active = b.activity.enter();
        let current: Live = Arc::new(move || r.lease.is_current());
        self.spawn(async move{
            let _active = active;
            let result=async{
                if !current(){return Err("Presentation changed".into());}
                match &r.action {
                    Action::Start|Action::StartSuggested(_)=>{
                        let settings=super::projections::settings(&b).await?;
                        let request=json!({"deviceName":settings["microphone"].as_str().filter(|s|!s.is_empty()&&*s!="system_default"),"smartAuto":if settings["smartAutoMicrophoneEnabled"]==true{json!({"approvedDeviceIds":settings["smartAutoApprovedDeviceIds"],"preferredDeviceIds":settings["smartAutoPreferredDeviceIds"],"allowContinuity":settings["smartAutoAllowContinuity"],"requireRecentSignal":settings["smartAutoProbeEnabled"]})}else{Value::Null},"retainAudio":settings["meetingRetainAudio"],"retentionDays":settings["meetingRetentionDays"].as_u64().filter(|n|*n>0),"maxSessions":settings["meetingMaxSessions"],"echoCancellation":settings["meetingEchoCancellationEnabled"],"diarization":settings["meetingDiarization"],"suggestionToken":if let Action::StartSuggested(token)=&r.action{Some(token)}else{None}});
                        let session=b.effect("start_meeting",json!({"request":request}),current.clone()).await?;
                        if current(){b.meetings.selection.lock().unwrap_or_else(|p|p.into_inner()).id=session["id"].as_str().map(String::from);}
                    },
                    Action::Stop{generation}=>b.services.stop_meeting(*generation).await?,
                    Action::Read|Action::List{..}|Action::Select(_)=>{},
                    Action::Delete(id) => {
                        b.retire_player();
                        b.effect("delete_meeting", json!({"id":id}), current.clone()).await?;
                        let mut selection=b.meetings.selection.lock().unwrap_or_else(|p|p.into_inner());
                        if selection.id.as_ref()==Some(id) { selection.id=None; selection.audio=Audio::default(); selection.calendar=Calendar::Closed; selection.bundle=None; }
                    },
                    Action::DeleteAll => {
                        b.retire_player();
                        b.effect("delete_all_meetings", json!({}), current.clone()).await?;
                        let mut selection=b.meetings.selection.lock().unwrap_or_else(|p|p.into_inner());
                        selection.id=None; selection.audio=Audio::default(); selection.calendar=Calendar::Closed; selection.bundle=None;
                    },
                    Action::Back=>{b.pause_player();b.discard_calendar();b.meetings.selection.lock().unwrap_or_else(|p|p.into_inner()).id=None;b.navigate(super::backend::Route::Home);},
                    Action::Playback{id,action}=>b.player_action(id.clone(),*action)?,
                    Action::CalendarLookup(id)=>{
                        let calendar=lookup_calendar(&b,id,&current).await;
                        if current(){b.meetings.selection.lock().unwrap_or_else(|p|p.into_inner()).calendar=calendar;}
                    },
                    Action::CalendarApply{id,token}=>{
                        let applied=b.effect("apply_meeting_calendar_event",json!({"sessionId":id,"selectionToken":token}),current.clone()).await;
                        if current(){
                            let (calendar,notice)=apply_outcome(applied.is_ok());
                            let mut selection=b.meetings.selection.lock().unwrap_or_else(|p|p.into_inner());
                            selection.calendar=calendar;
                            selection.notice=notice;
                        }
                    },
                    Action::Copy{id,format}=>{let text=b.effect("get_meeting_review_export",json!({"id":id,"format":format.wire()}),current.clone()).await?;if current(){b.services.copy_text(text.as_str().ok_or("Meeting export unavailable")?)?;}},
                    Action::Export{id,format}|Action::BundleExport{id,format}=>{
                        if let Some(path)=b.panel(json!({"kind":"save","name":format!("murmur-meeting.{}",if matches!(r.action,Action::BundleExport{..}){"zip"}else{match format {Format::Markdown=>"md",Format::Text=>"txt",_=>format.wire()}})}),current.clone()).await? {
                            let command=if matches!(r.action,Action::BundleExport{..}){"save_meeting_bundle_export"}else{"save_meeting_review_export"};
                            b.effect(command,json!({"id":id,"format":format.wire(),"path":path}),current.clone()).await?;
                        }
                    },
                    Action::BundlePreview(id)=>{let v=b.effect("get_meeting_bundle_preview",json!({"id":id}),current.clone()).await?;if current(){b.meetings.selection.lock().unwrap_or_else(|p|p.into_inner()).bundle=Some(Bundle{audio_segments:n(&v,"audioSegments"),audio_bytes:n(&v,"audioBytes"),unavailable:n(&v,"unavailableSegments"),max_bytes:n(&v,"maxAudioBytes")});}},
                    action=>if let Some((command,args))=action.command(){b.effect(command,args,current.clone()).await?;}
                }
                read(b.clone()).await
            }.await;
            if current()&&!inbox.is_closed(){let _=inbox.publish(Event::Reply{ticket:r.ticket,result:result.map_err(|_|Problem::Unavailable)});}
        });
    }
}
