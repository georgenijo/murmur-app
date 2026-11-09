//! Stable row identities and effects serialized with the existing Clear owner.
use super::{
    backend::{Backend, Route},
    import_queue::{self, Engine, Fut},
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
        let paths=self.os.panel(json!({"kind":"open","multiple":true,"title":"Transcribe audio or video","extensions":import_queue::EXTENSIONS}),current.clone()).await?;
        if let Some(paths) = paths.and_then(|v| v.as_array().cloned()) {
            self.transcribe_paths(paths, current);
        }
        Ok(())
    }
    /// Queue every supported path (no cap) and start the sequential drain if
    /// it is idle. Per-file failures are recorded on the queue, not returned.
    /// `current` only guards submission; once queued, an import is a
    /// background job that outlives the section that started it.
    pub fn transcribe_paths(&self, paths: Vec<Value>, current: Live) {
        if !current() {
            return;
        }
        let paths: Vec<String> = paths
            .iter()
            .filter_map(|p| p.as_str().map(str::to_owned))
            .collect();
        let run = {
            let mut queue = self
                .events
                .imports
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            queue.enqueue(&paths);
            queue.claim()
        };
        if run {
            let b = self.clone();
            self.spawn(async move {
                import_queue::drain(&b.events.imports, &b, operation_id).await;
            });
        }
    }
    /// Cancel a queued or active import from its toast.
    pub fn cancel_import(&self, id: u64) {
        let cancelled = self
            .events
            .imports
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .cancel(id);
        if let Some(Some(operation)) = cancelled {
            let b = self.clone();
            self.spawn(async move { Engine::cancel(&b, operation).await });
        }
    }
}
fn job() -> Live {
    Arc::new(|| true)
}
fn operation_id() -> String {
    uuid::Uuid::new_v4().to_string()
}
impl Engine for Backend {
    fn transcribe(&self, path: String, operation: String) -> Fut<'_, Result<(), String>> {
        Box::pin(async move {
            self.effect(
                "transcribe_file",
                json!({"filePath":path,"operationId":operation}),
                job(),
            )
            .await
            .map(|_| ())
        })
    }
    fn accept(&self, operation: String) -> Fut<'_, Result<(), String>> {
        Box::pin(async move {
            self.effect(
                "accept_file_transcription",
                json!({"operationId":operation}),
                job(),
            )
            .await
            .map(|_| ())
        })
    }
    fn cancel(&self, operation: String) -> Fut<'_, ()> {
        Box::pin(async move {
            let _ = self
                .call(
                    "cancel_file_transcription",
                    json!({"operationId":operation}),
                )
                .await;
        })
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
                        let text=if markdown {render(std::slice::from_ref(&entry),Format::Markdown)?}else{entry.text.clone()};
                        b.services.copy_history(r.ticket.revision,vec![entry.target()],text,current.clone())?;return Ok(Outcome::Copied);
                    },
                    RequestKind::Export{entries,format,save}=>{
                        let text=render(&entries,format)?;let targets=entries.iter().map(Entry::target).collect();
                        if !save {b.services.copy_history(r.ticket.revision,targets,text,current.clone())?;return Ok(Outcome::Copied);}
                        let Some(path)=b.panel(json!({"kind":"save","name":format!("murmur-history.{}",format.extension()),"extensions":[format.extension()]}),current.clone()).await? else{return Ok(Outcome::Cancelled);};
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
                        });let _=std::fs::remove_file(cleanup);result?;return Ok(Outcome::Saved);
                    },
                    RequestKind::Pin{entry}=>b.services.mutate_history(r.ticket.revision,murmur_core::completion::Command::History(murmur_core::completion::HistoryMutation::Pin{target:entry.target()}))?,
                    RequestKind::Delete{entries,clear_all}=>{
                        let command=if clear_all{murmur_core::completion::Command::Clear{destination:murmur_core::completion::Destination::History,at:chrono::Utc::now().timestamp_millis().max(0) as u64}}else{murmur_core::completion::Command::History(murmur_core::completion::HistoryMutation::Delete{targets:entries.iter().map(Entry::target).collect()})};
                        b.services.mutate_history(r.ticket.revision,command)?;
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
fn render(entries: &[Entry], format: Format) -> Result<String, String> {
    let refs = entries.iter().collect::<Vec<_>>();
    murmur_core::history::export(&refs, format, &chrono::Local::now().to_rfc3339(), |ms| {
        chrono::DateTime::from_timestamp_millis(ms)
            .map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d %H:%M")
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
}
