//! Content-free smart-destination presentation. The service publishes only a
//! status and an application name; this tracker owns the shipped generation
//! rules (newer generation resets, cancelled generations stay hidden, idle
//! clears) so Home and the overlay read one projection.
use serde_json::Value;

const MAX_APP_NAME: usize = 80;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    Current,
    Recent,
    Unavailable,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Selection {
    status: Status,
    app: Option<String>,
}
#[derive(Default)]
pub struct Destination {
    generation: u64,
    cancelled: u64,
    idle: bool,
    selection: Option<Selection>,
}

fn generation(payload: &Value) -> Option<u64> {
    payload["recordingId"]
        .as_u64()
        .filter(|g| *g > 0 && *g <= (1 << 53))
}
fn app_name(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|s| {
            !s.is_empty() && s.chars().count() <= MAX_APP_NAME && !s.chars().any(char::is_control)
        })
        .map(String::from)
}

impl Destination {
    pub fn event(&mut self, event: &str, payload: &str) {
        if !matches!(
            event,
            "dictation-generation-started"
                | "smart-destination-selected"
                | "recording-cancelled"
                | "recording-status-changed"
        ) {
            return;
        }
        let Ok(value) = serde_json::from_str::<Value>(payload) else {
            return;
        };
        match event {
            "recording-status-changed" => {
                self.idle = value == "idle";
                if self.idle {
                    self.selection = None;
                }
            }
            "dictation-generation-started" => {
                if let Some(id) = generation(&value).filter(|id| *id > self.generation) {
                    self.generation = id;
                    self.selection = None;
                }
            }
            "recording-cancelled" => {
                if let Some(id) = generation(&value) {
                    self.cancelled = self.cancelled.max(id);
                }
                self.selection = None;
            }
            _ => {
                let Some(id) = generation(&value) else {
                    return;
                };
                let status = match value["status"].as_str() {
                    Some("current") => Status::Current,
                    Some("recent") => Status::Recent,
                    Some("unavailable") => Status::Unavailable,
                    _ => return,
                };
                if self.idle || id < self.generation || id <= self.cancelled {
                    return;
                }
                self.generation = id;
                self.selection = Some(Selection {
                    status,
                    app: (status != Status::Unavailable)
                        .then(|| app_name(&value["appName"]))
                        .flatten(),
                });
            }
        }
    }
    /// Shipped wording. Callers show it only while recording or processing.
    pub fn label(&self) -> Option<String> {
        let selection = self.selection.as_ref()?;
        Some(match (&selection.status, &selection.app) {
            (Status::Unavailable, _) => "Clipboard only".into(),
            (_, Some(app)) => format!("To {app}"),
            (Status::Current, None) => "Current text field".into(),
            (Status::Recent, None) => "Recent text field".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn select(d: &mut Destination, id: u64, status: &str, app: Option<&str>) {
        d.event(
            "smart-destination-selected",
            &serde_json::json!({"recordingId": id, "status": status, "appName": app}).to_string(),
        );
    }
    #[test]
    fn service_payload_names_the_application_or_the_fallback() {
        let mut d = Destination::default();
        select(&mut d, 1, "current", Some("Notes"));
        assert_eq!(d.label().as_deref(), Some("To Notes"));
        select(&mut d, 1, "recent", None);
        assert_eq!(d.label().as_deref(), Some("Recent text field"));
        select(&mut d, 1, "current", None);
        assert_eq!(d.label().as_deref(), Some("Current text field"));
        select(&mut d, 1, "unavailable", Some("Notes"));
        assert_eq!(d.label().as_deref(), Some("Clipboard only"));
    }
    #[test]
    fn invalid_names_and_payloads_never_reach_presentation() {
        let mut d = Destination::default();
        select(&mut d, 1, "current", Some("No\u{7}te"));
        assert_eq!(d.label().as_deref(), Some("Current text field"));
        select(&mut d, 1, "current", Some(&"x".repeat(81)));
        assert_eq!(d.label().as_deref(), Some("Current text field"));
        select(&mut d, 1, "current", Some("Notes"));
        d.event("smart-destination-selected", "not json");
        select(&mut d, 0, "recent", None);
        select(&mut d, 1, "bogus", None);
        assert_eq!(d.label().as_deref(), Some("To Notes"));
    }
    #[test]
    fn stale_cancelled_and_idle_selections_are_hidden() {
        let mut d = Destination::default();
        d.event("dictation-generation-started", r#"{"recordingId":5}"#);
        select(&mut d, 4, "current", Some("Old"));
        assert_eq!(d.label(), None);
        select(&mut d, 5, "current", Some("Notes"));
        d.event("recording-cancelled", r#"{"recordingId":5}"#);
        assert_eq!(d.label(), None);
        select(&mut d, 5, "current", Some("Late"));
        assert_eq!(d.label(), None);
        d.event("dictation-generation-started", r#"{"recordingId":6}"#);
        select(&mut d, 6, "recent", Some("Mail"));
        assert_eq!(d.label().as_deref(), Some("To Mail"));
        d.event("dictation-generation-started", r#"{"recordingId":7}"#);
        assert_eq!(d.label(), None);
        select(&mut d, 7, "current", Some("Mail"));
        d.event("recording-status-changed", r#""idle""#);
        assert_eq!(d.label(), None);
        select(&mut d, 7, "current", Some("Late"));
        assert_eq!(d.label(), None);
        d.event("recording-status-changed", r#""recording""#);
        select(&mut d, 8, "current", Some("Next"));
        assert_eq!(d.label().as_deref(), Some("To Next"));
    }
}
