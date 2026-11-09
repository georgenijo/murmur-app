use serde_json::{Value, json};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    Show,
    Diagnostics,
    About,
    Settings,
    CheckUpdates,
    PasteLast,
    CycleMode,
    NextMode { revision: u64, id: Option<String> },
    ToggleDisabled,
    Quit,
}
impl Intent {
    pub fn parse(id: &str) -> Option<Self> {
        Some(match id {
            "show" => Self::Show,
            "diagnostics" => Self::Diagnostics,
            "about" => Self::About,
            "settings" => Self::Settings,
            "updates" => Self::CheckUpdates,
            "paste-last" => Self::PasteLast,
            "cycle-mode" => Self::CycleMode,
            "disabled" => Self::ToggleDisabled,
            "quit" => Self::Quit,
            _ => {
                let (revision, id) = id.strip_prefix("next:")?.split_once(':')?;
                Self::NextMode {
                    revision: revision.parse().ok()?,
                    id: (!id.is_empty()).then(|| id.into()),
                }
            }
        })
    }
}
#[derive(Clone, Default, PartialEq, Eq)]
pub struct TrayState {
    pub revision: u64,
    pub mode: String,
    pub mode_source: crate::overlay::model::ModeSource,
    pub available_modes: Vec<(String, String)>,
    pub pending_mode: Option<String>,
    pub disabled: bool,
    pub update: Option<String>,
}
impl TrayState {
    pub fn menu(&self) -> Value {
        let mut modes:Vec<Value>=self.available_modes.iter().map(|(id,name)|json!({
            "id":format!("next:{}:{id}",self.revision),"title":format!("Next recording: {name}"),"checked": self.pending_mode.as_ref()==Some(id)
        })).collect();
        modes.push(json!({"id":format!("next:{}:",self.revision),"title":"Clear next recording override","disabled": self.pending_mode.is_none()}));
        json!([
            {"id":"show","title":"Show Murmur"},
            {"id":"updates","title":self.update.as_ref().map(|v|format!("Update Murmur to v{v}…")).unwrap_or_else(||"Check for Updates…".into())},
            {"id":"paste-last","title":"Paste Last / Retry Delivery"},
            {"id":"cycle-mode","title":format!("Mode: {}{}",self.mode,self.mode_source.tray_suffix())},
            {"title":"Next recording","items":modes},
            {"separator":true},
            {"id":"disabled","title":"Disable Murmur","checked":self.disabled},
            {"separator":true},
            {"id":"quit","title":"Quit Murmur"}
        ])
    }
    pub fn accepts(&self, intent: &Intent) -> bool {
        match intent {
            Intent::NextMode { revision, id } => {
                *revision == self.revision
                    && id
                        .as_ref()
                        .is_none_or(|id| self.available_modes.iter().any(|(key, _)| key == id))
            }
            _ => true,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_or_removed_next_recording_action_is_rejected() {
        let s = TrayState {
            revision: 7,
            available_modes: vec![("everyday".into(), "Everyday".into())],
            ..Default::default()
        };
        assert!(s.accepts(&Intent::parse("next:7:everyday").unwrap()));
        assert!(s.accepts(&Intent::parse("next:7:").unwrap()));
        assert!(!s.accepts(&Intent::parse("next:6:everyday").unwrap()));
        assert!(!s.accepts(&Intent::parse("next:7:removed").unwrap()));
        assert!(Intent::parse("next:bad:everyday").is_none());
        assert_eq!(s.menu()[4]["items"][1]["disabled"], true);
    }
}
