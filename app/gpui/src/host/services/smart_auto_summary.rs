//! Overlay Smart Auto line from the production `get_smart_auto_microphone_status`
//! reader, with the shipped overlay's states, reason wording and validation.
//! Device ids and labels stay local: only the label of a device the user's own
//! inventory names is shown, and an unknown id falls back to a generic noun.
use serde_json::{Value, json};

/// The request the dictation start path sends, or `None` when Smart Auto is off.
pub fn request(settings: &Value) -> Option<Value> {
    (settings["smartAutoMicrophoneEnabled"] == true).then(|| {
        json!({
            "approvedDeviceIds": settings["smartAutoApprovedDeviceIds"],
            "preferredDeviceIds": settings["smartAutoPreferredDeviceIds"],
            "allowContinuity": settings["smartAutoAllowContinuity"],
            "requireRecentSignal": settings["smartAutoProbeEnabled"],
        })
    })
}

/// Ready and probing summaries name a device, so only they need the inventory.
pub fn needs_inventory(status: Option<&Value>) -> bool {
    status.is_some_and(|s| matches!(s["state"].as_str(), Some("ready" | "probing")))
}

fn reason(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "current_verified" => "already active with recent signal",
        "previous_verified_rollback" => "previously active with recent signal",
        "preferred_approved" => "your preferred included microphone",
        "approved_macos_default" => "an included microphone that matches the macOS default",
        "approved_external_fallback" => "the first available included external microphone",
        "approved_continuity_fallback" => "the available included iPhone microphone",
        _ => return None,
    })
}

fn phase(phase: &str) -> Option<&'static str> {
    Some(match phase {
        "connecting" => "connecting",
        "verifying" => "checking signal",
        "stopping" => "finishing",
        _ => return None,
    })
}

fn bounded(value: &Value, max: usize) -> Option<&str> {
    value
        .as_str()
        .filter(|s| !s.is_empty() && s.chars().count() <= max)
}

fn device_name(devices: Option<&Value>, id: &str, fallback: &'static str) -> String {
    devices
        .and_then(|d| d["devices"].as_array())
        .and_then(|list| list.iter().find(|d| d["id"] == id))
        .and_then(|d| d["name"].as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

const BLOCKED: &str = "Smart Auto blocked. Open settings to retry, verify signal, or pin an input.";

/// `request` was `Some`. `status` is the command result (`None` when it failed)
/// and `devices` the input inventory. An unreadable or unsupported status is the
/// shipped "unavailable" state, which presents as blocked without a cooldown.
pub fn summary(status: Option<&Value>, devices: Option<&Value>) -> String {
    let Some(status) = status else {
        return BLOCKED.into();
    };
    match status["state"].as_str() {
        Some("ready") => {
            let valid = &status["validForMs"];
            let verified = matches!(
                status["reason"].as_str(),
                Some("current_verified" | "previous_verified_rollback")
            );
            let valid_ok = match valid.as_u64() {
                Some(ms) => (1..=120_000).contains(&ms),
                None => valid.is_null() && !verified,
            };
            match (
                bounded(&status["deviceId"], 4096),
                status["reason"].as_str().and_then(reason),
                valid_ok,
            ) {
                (Some(id), Some(reason), true) => format!(
                    "Smart Auto: {}. {reason}.",
                    device_name(devices, id, "verified microphone")
                ),
                _ => BLOCKED.into(),
            }
        }
        Some("probing") => match (
            bounded(&status["deviceId"], 4096),
            status["phase"].as_str().and_then(phase),
            status["remainingMs"].as_u64().filter(|ms| *ms <= 75_000),
        ) {
            (Some(id), Some(phase), Some(_)) => format!(
                "Smart Auto checking {}: {phase}",
                device_name(devices, id, "included microphone")
            ),
            _ => BLOCKED.into(),
        },
        Some("blocked") => {
            let message_ok = bounded(&status["message"], 1024).is_some();
            let retry = &status["retryAfterMs"];
            let retry_ok =
                retry.is_null() || retry.as_u64().is_some_and(|ms| (1..=900_000).contains(&ms));
            match (message_ok && retry_ok, retry.is_null()) {
                (true, false) => "Smart Auto blocked. Waiting for cooldown.".into(),
                _ => BLOCKED.into(),
            }
        }
        _ => BLOCKED.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory() -> Value {
        json!({"devices":[{"id":"synthetic-built-in","name":"Synthetic Microphone"}]})
    }

    #[test]
    fn request_follows_the_production_dictation_start_shape() {
        let settings = json!({
            "smartAutoMicrophoneEnabled": true,
            "smartAutoApprovedDeviceIds": ["a"],
            "smartAutoPreferredDeviceIds": ["b"],
            "smartAutoAllowContinuity": false,
            "smartAutoProbeEnabled": true,
        });
        assert_eq!(
            request(&settings),
            Some(json!({
                "approvedDeviceIds": ["a"],
                "preferredDeviceIds": ["b"],
                "allowContinuity": false,
                "requireRecentSignal": true,
            }))
        );
        assert_eq!(
            request(&json!({"smartAutoMicrophoneEnabled": false})),
            None,
            "Smart Auto off shows nothing"
        );
        assert_eq!(request(&json!({})), None);
    }

    #[test]
    fn ready_probing_and_blocked_each_have_a_distinct_shipped_summary() {
        let devices = inventory();
        let ready = json!({"state":"ready","deviceId":"synthetic-built-in","reason":"preferred_approved","validForMs":30_000});
        assert_eq!(
            summary(Some(&ready), Some(&devices)),
            "Smart Auto: Synthetic Microphone. your preferred included microphone."
        );
        let probing = json!({"state":"probing","deviceId":"synthetic-built-in","phase":"verifying","remainingMs":9_000});
        assert_eq!(
            summary(Some(&probing), Some(&devices)),
            "Smart Auto checking Synthetic Microphone: checking signal"
        );
        let cooldown = json!({"state":"blocked","message":"Synthetic block","retryAfterMs":45_000});
        assert_eq!(
            summary(Some(&cooldown), None),
            "Smart Auto blocked. Waiting for cooldown."
        );
        let pinned = json!({"state":"blocked","message":"Synthetic block","retryAfterMs":null});
        assert_eq!(summary(Some(&pinned), None), BLOCKED);
    }

    #[test]
    fn unknown_devices_use_generic_names_and_never_echo_ids() {
        let ready = json!({"state":"ready","deviceId":"PRIVATE-SYNTHETIC-ID","reason":"current_verified","validForMs":10_000});
        let text = summary(Some(&ready), Some(&inventory()));
        assert!(text.contains("verified microphone"));
        assert!(!text.contains("PRIVATE"));
        let probing = json!({"state":"probing","deviceId":"PRIVATE-SYNTHETIC-ID","phase":"connecting","remainingMs":0});
        let text = summary(Some(&probing), None);
        assert!(text.contains("included microphone") && text.contains("connecting"));
        assert!(!text.contains("PRIVATE"));
    }

    #[test]
    fn unreadable_or_unsupported_status_is_the_blocked_unavailable_state() {
        assert_eq!(summary(None, None), BLOCKED);
        for bad in [
            json!({"state":"mystery"}),
            json!({"state":"ready","deviceId":"d","reason":"nope","validForMs":1000}),
            json!({"state":"ready","deviceId":"d","reason":"current_verified","validForMs":null}),
            json!({"state":"ready","deviceId":"d","reason":"preferred_approved","validForMs":120_001}),
            json!({"state":"ready","deviceId":"","reason":"preferred_approved","validForMs":null}),
            json!({"state":"probing","deviceId":"d","phase":"sleeping","remainingMs":1}),
            json!({"state":"probing","deviceId":"d","phase":"connecting","remainingMs":75_001}),
            json!({"state":"blocked","message":"","retryAfterMs":null}),
            json!({"state":"blocked","message":"x","retryAfterMs":0}),
        ] {
            assert_eq!(summary(Some(&bad), Some(&inventory())), BLOCKED, "{bad}");
        }
        // A fixed verified-reason-free ready state may be unbounded.
        let open = json!({"state":"ready","deviceId":"synthetic-built-in","reason":"preferred_approved","validForMs":null});
        assert!(summary(Some(&open), Some(&inventory())).starts_with("Smart Auto: Synthetic"));
    }

    #[test]
    fn only_device_naming_states_need_the_inventory() {
        assert!(needs_inventory(Some(&json!({"state":"ready"}))));
        assert!(needs_inventory(Some(&json!({"state":"probing"}))));
        assert!(!needs_inventory(Some(&json!({"state":"blocked"}))));
        assert!(!needs_inventory(None));
    }
}
