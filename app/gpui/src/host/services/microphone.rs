//! A revocable Settings-page meter. Capture admission and PCM disposal belong
//! to the existing preview owner; no monitor is started by a subscription.
use super::backend::Backend;
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[derive(Default)]
pub struct Preview {
    epoch: AtomicU64,
    pub id: AtomicU64,
    pub meter: std::sync::Mutex<Option<(u64, f32, f32)>>,
    pub check: std::sync::Mutex<Option<String>>,
}
impl Backend {
    pub fn preview_page(&self, page: Option<crate::settings::schema::Page>, _attachment: u64) {
        let epoch = self.preview.epoch.fetch_add(1, Ordering::SeqCst) + 1;
        self.preview.id.store(0, Ordering::SeqCst);
        self.services.main_hidden();
        if page.is_none() {
            self.navigate(super::backend::Route::Calibration {
                offset: None,
                finish: true,
            });
        }
        if page == Some(crate::settings::schema::Page::Shortcuts) {
            // Viewing the shortcuts page completes its checklist item.
            super::discovery::complete(self, "shortcuts");
        }
        if page != Some(crate::settings::schema::Page::Recording) {
            return;
        }
        let b = self.clone();
        self.spawn(async move {
            let live:super::os::Live={let p=b.preview.clone();Arc::new(move||p.epoch.load(Ordering::SeqCst)==epoch)};
            while live(){
                if let Ok(settings)=super::projections::settings(&b).await
                    && live()
                    && b.call("check_microphone_permission_status",json!({})).await.is_ok_and(|v|v=="granted")
                    && live()
                    && let Ok(status)=b.call("get_microphone_preview_status",json!({})).await
                    && status["state"]=="idle" {
                    let smart=(settings["smartAutoMicrophoneEnabled"]==true).then(||json!({
                        "approvedDeviceIds":settings["smartAutoApprovedDeviceIds"],"preferredDeviceIds":settings["smartAutoPreferredDeviceIds"],
                        "allowContinuity":settings["smartAutoAllowContinuity"],"requireRecentSignal":settings["smartAutoProbeEnabled"]}));
                    if let Ok(started)=b.effect("start_microphone_preview",json!({"deviceId":settings["microphone"].as_str().filter(|s|!s.is_empty()).unwrap_or("system_default"),"smartAuto":smart,"vadSensitivity":settings["vadSensitivity"]}),live.clone()).await
                        && let Some(id)=started["previewId"].as_u64(){
                        if live(){b.preview.id.store(id,Ordering::SeqCst);}else{let _=b.call("cancel_microphone_preview",json!({"previewId":id})).await;}
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            }
        });
    }
}
impl Backend {
    pub async fn start_microphone_check(&self, current: super::os::Live) -> Result<(), String> {
        let settings = super::projections::settings(self).await?;
        let id = uuid::Uuid::new_v4().to_string();
        *self.preview.check.lock().unwrap_or_else(|p| p.into_inner()) = Some(id.clone());
        let result=self.effect("run_microphone_startup_benchmark",json!({"request":{"runId":id,"deviceId":settings["microphone"].as_str().filter(|s|!s.is_empty()).unwrap_or("system_default"),"cycles":5}}),current.clone()).await;
        if self
            .preview
            .check
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            == Some(&id)
        {
            self.preview
                .check
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .take();
        }
        result.map(|_| ())
    }
    pub async fn cancel_microphone_check(&self) -> Result<(), String> {
        let id = self
            .preview
            .check
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone();
        if let Some(id) = id {
            self.call("cancel_microphone_startup_benchmark", json!({"runId":id}))
                .await?;
        }
        Ok(())
    }
}
