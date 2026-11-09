//! Meeting presentation and playback adapt the shared recording/store owners.
use super::{
    backend::{AccessMemory, Backend},
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
/// Service-owned runtime facts shown in the status strip. Everything comes
/// from `get_meeting_status`, the permission probe and the last access receipt.
struct Runtime {
    phase: Phase,
    permission: Permission,
    needs_relaunch: bool,
    failure: Option<&'static str>,
    echo_notice: Option<String>,
}
fn runtime(status: &Value, passive: &Value, access: &AccessMemory) -> Runtime {
    // A refused-tap receipt outranks the passive authorization probe.
    let permission = access.permission(passive);
    Runtime {
        phase: match status["phase"].as_str() {
            Some("starting") => Phase::Starting,
            Some("recording") => Phase::Recording,
            Some("stopping") => Phase::Stopping,
            Some("processing") => Phase::Processing,
            Some("failed") => Phase::Failed,
            _ => Phase::Idle,
        },
        permission: match permission.as_str() {
            Some("granted") => Permission::Granted,
            Some("denied") => Permission::Denied,
            Some("unsupported") => Permission::Unsupported,
            _ => Permission::Unknown,
        },
        needs_relaunch: access.needs_relaunch(),
        failure: status["errorCode"].as_str().map(error_message),
        echo_notice: echo_notice(&status["echoCancellation"]),
    }
}
/// Shipped `echoCancellationNotice`: only a recovering or bypassed echo canceller
/// is user-visible; off, starting and active are silent.
fn echo_notice(echo: &Value) -> Option<String> {
    match echo["state"].as_str()? {
        "recovering" => Some(format!(
            "Speaker echo reduction was interrupted. Murmur is keeping the original microphone audio while it recovers (attempt {} of {}).",
            n(echo, "attempt"),
            n(echo, "maxAttempts")
        )),
        "bypassed" => Some(
            "Speaker echo reduction is unavailable. Murmur is keeping the original microphone audio for the rest of this meeting."
                .into(),
        ),
        _ => None,
    }
}
/// The current status that may confirm a live tap. Only a held receipt and an
/// observed live tap warrant the extra read, and a failed read confirms nothing.
async fn confirm_live_tap<F, Fut>(held: bool, observed: &Value, fetch: F) -> Option<Value>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    if held && observed["systemAudioActive"] == true {
        fetch().await.ok()
    } else {
        None
    }
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
        // Taken before the status read, so a receipt recorded meanwhile survives it.
        let seen_receipt = b
            .system_audio_access
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .sequence();
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
        // `status` is older than the awaits above. Before it may clear the access
        // advice, confirm the same generation still has a live tap right now.
        let held = b
            .system_audio_access
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .has_receipt();
        let confirm =
            confirm_live_tap(held, &status, || b.call("get_meeting_status", json!({}))).await;
        let live = {
            let mut access = b
                .system_audio_access
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            access.supersede_confirmed(seen_receipt, &status, confirm.as_ref());
            runtime(&status, &permission, &access)
        };
        if busy {
            b.pause_player();
        }
        Ok(Snapshot {
            revision,
            phase: live.phase,
            recording_generation: n(&status, "generation"),
            recording_session: status["sessionId"].as_str().map(String::from),
            elapsed_ms: n(&status, "elapsedMs"),
            microphone_active: status["microphoneActive"] == true,
            system_audio_active: status["systemAudioActive"] == true,
            capture_busy: busy,
            permission: live.permission,
            needs_relaunch: live.needs_relaunch,
            failure: live.failure,
            echo_notice: live.echo_notice,
            meetings: rows,
            query: selection.query,
            total,
            detail,
            calendar: selection.calendar,
            bundle: selection.bundle,
            audio: selection.audio,
            notice: None,
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
                        // start_meeting only accepts startup. The tap opens afterwards and can still be
                        // refused, so relaunch advice is cleared only by a live capture in `read`.
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
                        let permission=b.effect("get_calendar_permission_status",json!({}),current.clone()).await?;
                        let permission=if permission=="notDetermined"{b.effect("request_calendar_permission",json!({}),current.clone()).await?}else{permission};
                        let calendar=if permission=="granted"{
                            let candidates=b.effect("get_meeting_calendar_events",json!({"sessionId":id}),current.clone()).await?;
                            Calendar::Choosing(candidates.as_array().into_iter().flatten().map(|c|Candidate{token:strv(c,"selectionToken"),title:strv(c,"title"),attendees:c["attendees"].as_array().into_iter().flatten().filter_map(Value::as_str).map(String::from).collect(),start_ms:n(c,"startMs") as i64,end_ms:n(c,"endMs") as i64}).collect())
                        }else{Calendar::Unavailable{message:"Calendar access is unavailable. Change access in System Settings.",denied:permission=="denied",unsupported:permission=="unsupported"}};
                        if current(){b.meetings.selection.lock().unwrap_or_else(|p|p.into_inner()).calendar=calendar;}
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
#[cfg(test)]
mod runtime_tests {
    use super::*;

    // `get_meeting_status` wire shape of `MeetingRuntimeStatus`, synthetic values only.
    fn status(phase: &str, error: Value, echo: Value) -> Value {
        json!({
            "generation": 7,
            "sessionId": "synthetic-session",
            "phase": phase,
            "elapsedMs": 65000,
            "microphoneActive": false,
            "systemAudioActive": false,
            "echoCancellation": echo,
            "errorCode": error,
        })
    }
    fn live_status(active: bool) -> Value {
        let mut status = status("recording", Value::Null, json!({"state":"off"}));
        status["microphoneActive"] = json!(true);
        status["systemAudioActive"] = json!(active);
        status
    }
    // `request_system_audio_permission` receipt, as the service serializes it.
    fn memory(permission: &str, relaunch: bool) -> AccessMemory {
        let mut memory = AccessMemory::default();
        memory.record(
            "request_system_audio_permission",
            &json!({"permission":permission,"captureReady":permission == "granted","audioFlowing":false,"needsRelaunch":relaunch}),
        );
        memory
    }

    #[test]
    fn failed_meeting_carries_the_service_error_and_does_not_look_busy() {
        for (code, expected) in [
            (
                "system_audio_permission_denied",
                "System Audio access was denied.",
            ),
            (
                "capture_setup_timeout",
                "Core Audio stalled while starting capture. Quit other audio capture apps and try again.",
            ),
            (
                "termination_unconfirmed",
                "The capture worker could not be confirmed stopped. Restart Murmur before recording again.",
            ),
            (
                "future_unknown_code",
                "Meeting capture stopped unexpectedly.",
            ),
        ] {
            let r = runtime(
                &status("failed", json!(code), json!({"state":"off"})),
                &json!("denied"),
                &AccessMemory::default(),
            );
            assert!(r.phase == Phase::Failed);
            assert!(!r.phase.busy(), "a failed meeting has released capture");
            assert_eq!(r.failure, Some(expected));
        }
    }

    #[test]
    fn healthy_and_idle_meetings_show_no_failure_or_echo_notice() {
        for (phase, expected) in [
            ("idle", Phase::Idle),
            ("starting", Phase::Starting),
            ("recording", Phase::Recording),
            ("stopping", Phase::Stopping),
            ("processing", Phase::Processing),
        ] {
            let r = runtime(
                &status(phase, Value::Null, json!({"state":"active"})),
                &json!("granted"),
                &AccessMemory::default(),
            );
            assert!(r.phase == expected);
            assert!(r.failure.is_none());
            assert!(r.echo_notice.is_none());
            assert!(r.permission == Permission::Granted);
            assert!(!r.needs_relaunch);
        }
    }

    #[test]
    fn echo_reduction_notices_match_the_service_state() {
        let notice = |echo| {
            runtime(
                &status("recording", Value::Null, echo),
                &json!("granted"),
                &AccessMemory::default(),
            )
            .echo_notice
        };
        for silent in ["off", "starting", "active"] {
            assert_eq!(notice(json!({"state":silent})), None);
        }
        assert_eq!(
            notice(json!({"state":"recovering","reason":"processorFailed","episode":2,"attempt":2,"maxAttempts":3})).as_deref(),
            Some("Speaker echo reduction was interrupted. Murmur is keeping the original microphone audio while it recovers (attempt 2 of 3).")
        );
        assert_eq!(
            notice(json!({"state":"bypassed","reason":"initializationFailed"})).as_deref(),
            Some(
                "Speaker echo reduction is unavailable. Murmur is keeping the original microphone audio for the rest of this meeting."
            )
        );
    }

    #[test]
    fn refused_tap_receipt_shows_denied_and_relaunch_over_a_granted_passive_probe() {
        // macOS lists the app as authorized, so the passive probe reports granted
        // while the tap refusal still needs a relaunch.
        let r = runtime(
            &status("idle", Value::Null, json!({"state":"off"})),
            &json!("granted"),
            &memory("denied", true),
        );
        assert!(r.needs_relaunch);
        assert!(r.permission == Permission::Denied);
        // A refusal without a relaunch fact still shows denied, not granted.
        let r = runtime(
            &status("idle", Value::Null, json!({"state":"off"})),
            &json!("granted"),
            &memory("denied", false),
        );
        assert!(!r.needs_relaunch);
        assert!(r.permission == Permission::Denied);
        // An accepted tap leaves the passive answer alone.
        let r = runtime(
            &status("idle", Value::Null, json!({"state":"off"})),
            &json!("granted"),
            &memory("granted", false),
        );
        assert!(r.permission == Permission::Granted);
    }

    #[test]
    fn unknown_receipt_stays_unknown_over_a_granted_passive_probe() {
        let mut access = memory("unknown", false);
        let idle = status("idle", Value::Null, json!({"state":"off"}));
        let r = runtime(&idle, &json!("granted"), &access);
        assert!(r.permission == Permission::Unknown);
        assert!(!r.needs_relaunch);
        // A live tap on the current generation is what makes it ready.
        let seen = access.sequence();
        let live = live_status(true);
        access.supersede_confirmed(seen, &live, Some(&live));
        let r = runtime(&live, &json!("granted"), &access);
        assert!(r.permission == Permission::Granted);
    }

    #[test]
    fn live_tap_from_a_superseded_generation_keeps_the_relaunch_advice() {
        let mut access = memory("denied", true);
        let seen = access.sequence();
        let observed = live_status(true);
        // By the time the read clears, a different generation owns the meeting.
        let mut newer = live_status(true);
        newer["generation"] = json!(8);
        newer["sessionId"] = json!("synthetic-next");
        access.supersede_confirmed(seen, &observed, Some(&newer));
        let r = runtime(&newer, &json!("granted"), &access);
        assert!(r.needs_relaunch);
        assert!(r.permission == Permission::Denied);
    }

    #[test]
    fn accepted_start_then_tap_denied_keeps_the_relaunch_advice_until_a_live_tap() {
        let mut access = memory("denied", true);
        let seen = access.sequence();
        // start_meeting accepted: the meeting is starting, then fails on the refused tap.
        for status in [
            status("starting", Value::Null, json!({"state":"off"})),
            status(
                "failed",
                json!("system_audio_permission_denied"),
                json!({"state":"off"}),
            ),
        ] {
            settle(&mut access, seen, &status, Ok(status.clone()));
            let r = runtime(&status, &json!("granted"), &access);
            assert!(r.needs_relaunch, "{}", status["phase"]);
            assert!(r.permission == Permission::Denied);
        }
        // Recording without a system tap proves nothing about System Audio.
        settle(
            &mut access,
            seen,
            &live_status(false),
            Ok(live_status(false)),
        );
        assert!(access.needs_relaunch());
        // Only a live system-audio tap supersedes the receipt.
        settle(&mut access, seen, &live_status(true), Ok(live_status(true)));
        let r = runtime(&live_status(true), &json!("granted"), &access);
        assert!(!r.needs_relaunch);
        assert!(r.permission == Permission::Granted);
    }

    /// Runs the production confirmation step of `read` and then its clear, with
    /// `fetched` standing in for the second `get_meeting_status` call. Returns how
    /// many times that call was made.
    fn settle(
        access: &mut AccessMemory,
        seen: u64,
        observed: &Value,
        fetched: Result<Value, String>,
    ) -> usize {
        let calls = std::cell::Cell::new(0);
        let held = access.has_receipt();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let confirm = rt.block_on(confirm_live_tap(held, observed, || {
            calls.set(calls.get() + 1);
            std::future::ready(fetched)
        }));
        access.supersede_confirmed(seen, observed, confirm.as_ref());
        calls.get()
    }

    #[test]
    fn a_failed_or_changed_confirmation_keeps_the_relaunch_advice() {
        // The first read saw a live tap, but the confirming read fails.
        let mut access = memory("denied", true);
        let seen = access.sequence();
        let calls = settle(
            &mut access,
            seen,
            &live_status(true),
            Err("Meeting status unavailable".into()),
        );
        assert_eq!(calls, 1);
        assert!(access.needs_relaunch());
        assert!(access.permission(&json!("granted")) == json!("denied"));
        // The confirming read shows a different generation.
        let mut other = live_status(true);
        other["generation"] = json!(8);
        settle(&mut access, seen, &live_status(true), Ok(other));
        assert!(access.needs_relaunch());
        // The confirming read lost the tap.
        settle(
            &mut access,
            seen,
            &live_status(true),
            Ok(live_status(false)),
        );
        assert!(access.needs_relaunch());
        // The same generation still live confirms and clears.
        settle(&mut access, seen, &live_status(true), Ok(live_status(true)));
        assert!(!access.needs_relaunch());
    }

    #[test]
    fn the_confirming_read_only_happens_for_a_held_receipt_and_a_live_tap() {
        let mut none = AccessMemory::default();
        let seen = none.sequence();
        let calls = settle(&mut none, seen, &live_status(true), Ok(live_status(true)));
        assert_eq!(calls, 0, "no receipt, nothing to confirm");
        let mut access = memory("denied", true);
        let seen = access.sequence();
        let calls = settle(
            &mut access,
            seen,
            &live_status(false),
            Ok(live_status(true)),
        );
        assert_eq!(calls, 0, "no live tap observed, nothing to confirm");
        assert!(access.needs_relaunch());
    }
}
