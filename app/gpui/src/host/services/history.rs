//! Stable row identities and effects serialized with the existing Clear owner.
use super::{
    backend::{Backend, Route},
    os::Live,
};
use crate::history::{host::*, inbox::Inbox, model::*};
use serde_json::{Value, json};
use std::{
    io::Write,
    os::unix::fs::OpenOptionsExt,
    sync::{Arc, Mutex},
};
#[derive(Default)]
pub struct Keys(Mutex<(u64, Vec<(u64, Value)>)>);
impl Keys {
    fn project(&self, rows: Vec<Value>) -> Vec<Entry> {
        let mut state = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let old = std::mem::take(&mut state.1);
        let mut used = std::collections::BTreeSet::new();
        let mut out = Vec::new();
        for row in rows {
            let matched = old
                .iter()
                .find(|(key, v)| !used.contains(key) && *v == row)
                .or_else(|| {
                    old.iter().find(|(key, v)| {
                        !used.contains(key)
                            && v["id"] == row["id"]
                            && v["timestamp"] == row["timestamp"]
                    })
                });
            let key = if let Some((key, _)) = matched {
                *key
            } else {
                state.0 = state.0.saturating_add(1);
                state.0
            };
            used.insert(key);
            if let Some(entry) = Entry::project(key, row.clone()) {
                state.1.push((key, row));
                out.push(entry);
            }
        }
        out
    }
}
impl Backend {
    async fn history_read(&self) -> Result<(u64, Snapshot), String> {
        let (epoch, snapshot) = self.services.history_snapshot().await?;
        let settings = super::projections::settings(self).await?;
        Ok((
            epoch,
            Snapshot {
                entries: self.history_keys.project(snapshot.history),
                retain_history: settings["retainHistory"] != false,
                retry_import: snapshot.retry_notice,
            },
        ))
    }
    pub async fn transcribe_files(&self, current: Live) -> Result<(), String> {
        let paths=self.os.panel(json!({"kind":"open","multiple":true,"title":"Transcribe audio or video","extensions":["wav","mp3","m4a","mp4","mov","flac","aac","ogg"]}),current.clone()).await?;
        if let Some(paths) = paths.and_then(|v| v.as_array().cloned()) {
            self.transcribe_paths(paths, current).await?;
        }
        Ok(())
    }
    pub async fn transcribe_paths(&self, paths: Vec<Value>, current: Live) -> Result<(), String> {
        for path in paths.into_iter().take(64) {
            if !current() {
                break;
            }
            let operation = uuid::Uuid::new_v4().to_string();
            self.effect(
                "transcribe_file",
                json!({"filePath":path,"operationId":operation}),
                current.clone(),
            )
            .await?;
            if !current() {
                let _ = self
                    .call(
                        "cancel_file_transcription",
                        json!({"operationId":operation}),
                    )
                    .await;
                break;
            }
            self.effect(
                "accept_file_transcription",
                json!({"operationId":operation}),
                current.clone(),
            )
            .await?;
        }
        Ok(())
    }
}
impl Host for Backend {
    fn subscribe(
        &self,
        c: murmur_core::events::Consumer,
        a: u64,
        inbox: Inbox,
    ) -> Box<dyn Subscription> {
        let (send, mut receive) = tokio::sync::mpsc::channel(1);
        let stop = self.events.listen(
            c,
            Arc::new(move |event, _| {
                if matches!(
                    event,
                    "history-changed" | "completion-storage-error" | "dictation-settings-changed"
                ) {
                    let _ = send.try_send(());
                }
            }),
        );
        let b = self.clone();
        self.spawn(async move {
            while receive.recv().await.is_some() && !inbox.is_closed() {
                if let Ok((epoch, snapshot)) = b.history_read().await {
                    let _ = inbox.publish(Event::Changed {
                        attachment: a,
                        revision: epoch,
                        clear_epoch: epoch,
                        snapshot,
                    });
                }
            }
        });
        Box::new(stop)
    }
    fn request(&self, r: Request, inbox: Inbox) {
        if !r.lease.is_current() || inbox.is_closed() {
            return;
        }
        let b = self.clone();
        let current: Live = Arc::new(move || r.lease.is_current());
        self.spawn(async move {
            if matches!(r.kind,RequestKind::Read){let result=b.history_read().await;if current()&&!inbox.is_closed(){let (revision,result)=match result{Ok((epoch,s))=>(epoch,Ok(s)),Err(_)=>(r.ticket.revision,Err(Problem::Unavailable))};let _=inbox.publish(Event::Read{ticket:r.ticket,revision,clear_epoch:revision,result});}return;}
            let result:Result<Outcome,String>=async {
                if !current(){return Ok(Outcome::Cancelled);}
                match r.kind {
                    RequestKind::Read=>{},
                    RequestKind::CopyRow{entry,markdown}=>{
                        let text=if markdown {render(std::slice::from_ref(&entry),Format::Markdown,chrono::Local::now())?}else{entry.text.clone()};
                        b.services.copy_history(r.ticket.revision,vec![entry.target()],text,current.clone())?;return Ok(Outcome::Copied);
                    },
                    RequestKind::Export{entries,format,save}=>{
                        // The shipped panel names these failures; a cancel is silent.
                        let count=entries.len();
                        let exported:Result<Outcome,String>=async {
                            let text=render(&entries,format,chrono::Local::now())?;let targets=entries.iter().map(Entry::target).collect();
                            if !save {b.services.copy_history(r.ticket.revision,targets,text,current.clone())?;return Ok(Outcome::exported(false,count));}
                            let Some(path)=b.panel(json!({"kind":"save","name":export_name(format,chrono::Local::now()),"extensions":[format.extension()]}),current.clone()).await? else{return Ok(Outcome::Cancelled);};
                            if !current(){return Ok(Outcome::Cancelled);}
                            let destination=std::path::PathBuf::from(path);let parent=destination.parent().ok_or("Invalid export destination")?;
                            let staged=parent.join(format!(".murmur-history-{}.pending",uuid::Uuid::new_v4()));
                            let mut file=std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&staged).map_err(|_|"Export unavailable")?;
                            let prepared=file.write_all(text.as_bytes()).and_then(|_|file.sync_all());drop(file);
                            if prepared.is_err(){let _=std::fs::remove_file(&staged);return Err("Export unavailable".into());}
                            let cleanup=staged.clone();let live=current.clone();
                            let result=b.services.publish_history(r.ticket.revision,targets,move||{
                                if !live(){return Err(murmur_core::completion::Error::IdentityConflict);}
                                let parent=destination.parent().ok_or(murmur_core::completion::Error::Io)?;
                                std::fs::rename(&staged,&destination).and_then(|_| std::fs::File::open(parent)?.sync_all()).map_err(|_|murmur_core::completion::Error::Io)
                            });let _=std::fs::remove_file(cleanup);result?;Ok(Outcome::exported(true,count))
                        }.await;
                        return Ok(exported.unwrap_or(export_failure(save)));
                    },
                    RequestKind::Pin{entry}=>b.services.mutate_history(r.ticket.revision,murmur_core::completion::Command::History(murmur_core::completion::HistoryMutation::Pin{target:entry.target()}))?,
                    RequestKind::Delete{entries,row,clear_all}=>{
                        let count=entries.len();
                        let command=if clear_all{murmur_core::completion::Command::Clear{destination:murmur_core::completion::Destination::History,at:chrono::Utc::now().timestamp_millis().max(0) as u64}}else{murmur_core::completion::Command::History(murmur_core::completion::HistoryMutation::Delete{targets:entries.iter().map(Entry::target).collect()})};
                        b.services.mutate_history(r.ticket.revision,command)?;
                        return Ok(Outcome::deleted(row,clear_all,count));
                    },
                    RequestKind::CorrectAndTeach{entry}=>b.navigate(Route::Teach(entry)),
                    RequestKind::TranscribeFile=>b.transcribe_files(current.clone()).await?,
                    RequestKind::RetryImport=>{super::application::bootstrap(b.clone());}
                }Ok(Outcome::Opened)
            }.await;
            if current()&&!inbox.is_closed(){let _=inbox.publish(Event::Action{ticket:r.ticket,outcome:result.unwrap_or(Outcome::Failed)});}
        });
    }
}
/// The shipped panel's `Export to clipboard failed.` and save-failure receipts.
fn export_failure(save: bool) -> Outcome {
    if save {
        Outcome::SaveFailed
    } else {
        Outcome::ExportCopyFailed
    }
}
/// `murmur-history-YYYY-MM-DD-HHMM.ext`, the shipped save-panel suggestion.
fn export_name(format: Format, now: chrono::DateTime<chrono::Local>) -> String {
    format!(
        "murmur-history-{}.{}",
        now.format("%Y-%m-%d-%H%M"),
        format.extension()
    )
}
/// Shipped JSON exports carry a UTC ISO stamp (`toISOString`); Markdown headers
/// carry the local `YYYY-MM-DD HH:MM:SS` used for every exported row.
fn exported_at(format: Format, now: chrono::DateTime<chrono::Local>) -> String {
    match format {
        Format::Json => now
            .with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        _ => now.format("%Y-%m-%d %H:%M:%S").to_string(),
    }
}
fn render(
    entries: &[Entry],
    format: Format,
    now: chrono::DateTime<chrono::Local>,
) -> Result<String, String> {
    let refs = entries.iter().collect::<Vec<_>>();
    murmur_core::history::export(&refs, format, &exported_at(format, now), |ms| {
        chrono::DateTime::from_timestamp_millis(ms)
            .map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M:%S")
                    .to_string()
            })
            .unwrap_or_default()
    })
    .ok_or_else(|| "Export exceeds the supported size".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_legacy_ids_and_pin_changes_preserve_distinct_keys() {
        let keys = Keys::default();
        let a = json!({"id":"old","timestamp":1,"duration":0,"text":"synthetic a"});
        let c = json!({"id":"old","timestamp":1,"duration":0,"text":"synthetic b"});
        let first = keys.project(vec![a.clone(), c.clone()]);
        assert_ne!(first[0].key, first[1].key);
        let mut pinned = a;
        pinned["pinned"] = json!(true);
        let next = keys.project(vec![pinned, c]);
        assert_eq!(first[0].key, next[0].key);
        assert_eq!(first[1].key, next[1].key);
    }
    fn local(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> chrono::DateTime<chrono::Local> {
        use chrono::TimeZone;
        chrono::Local
            .with_ymd_and_hms(y, mo, d, h, mi, s)
            .single()
            .expect("unambiguous synthetic local time")
    }
    #[test]
    fn save_panel_name_matches_shipped_stamp_and_extension() {
        let now = local(2027, 1, 3, 9, 5, 42);
        assert_eq!(
            export_name(Format::Markdown, now),
            "murmur-history-2027-01-03-0905.md"
        );
        assert_eq!(
            export_name(Format::Json, now),
            "murmur-history-2027-01-03-0905.json"
        );
        assert_eq!(
            export_name(Format::Text, now),
            "murmur-history-2027-01-03-0905.txt"
        );
    }
    #[test]
    fn exports_carry_the_shipped_stamps_with_seconds() {
        let now = local(2027, 1, 3, 9, 5, 42);
        let row = Entry::project(
            1,
            json!({"id":"a","timestamp":1_800_000_000_000_i64,"duration":65.,"text":"synthetic text"}),
        )
        .unwrap();
        let markdown = render(std::slice::from_ref(&row), Format::Markdown, now).unwrap();
        assert!(markdown.contains("Exported 2027-01-03 09:05:42 · 1 entry\n"));
        let heading = markdown.lines().find(|l| l.starts_with("## ")).unwrap();
        // `## YYYY-MM-DD HH:MM:SS`, local to the machine, with seconds.
        assert_eq!(heading.len(), 3 + 19, "{heading}");
        let text = render(std::slice::from_ref(&row), Format::Text, now).unwrap();
        assert_eq!(text.find(" · "), Some(20), "{text}");
        let json: Value =
            serde_json::from_str(&render(std::slice::from_ref(&row), Format::Json, now).unwrap())
                .unwrap();
        let stamp = json["exportedAt"].as_str().unwrap();
        assert!(stamp.ends_with('Z') && stamp.len() == 24, "{stamp}");
        assert_eq!(json["count"], 1);
    }
    #[test]
    fn failed_exports_use_the_shipped_receipts() {
        assert_eq!(
            export_failure(false).message().as_deref(),
            Some("Export to clipboard failed.")
        );
        assert_eq!(
            export_failure(true).message().as_deref(),
            Some("Could not save the export.")
        );
    }
}
