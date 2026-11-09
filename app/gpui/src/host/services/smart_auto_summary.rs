//! Overlay Smart Auto state from the production `get_smart_auto_microphone_status`
//! reader, with the shipped overlay's states, reason wording, validation and
//! deadlines. Device ids and labels stay local: only the label of a device the
//! user's own inventory names is shown, and an unknown id falls back to a
//! generic noun.
use crate::overlay::smart_auto::{SmartAuto, SmartAutoState};
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

/// Ready and probing states name a device, so only they need the inventory.
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

/// What one read of the service means for the overlay.
#[derive(Debug, PartialEq, Eq)]
pub enum Projection {
    Show(SmartAuto),
    /// The read had already run out by the time it completed (or reported zero
    /// time left), so it says nothing about now. The shipped hook rereads once
    /// and, if that is also spent, treats Smart Auto as unavailable.
    Stale,
}

/// `request` was `Some`. `status` is the command result (`None` when it failed)
/// and `devices` the input inventory. Deadlines count from `requested_ms`, when
/// the service was asked, so read latency is already spent; a reread's blocked
/// cooldown instead counts from `completed_ms`, as the shipped deadline reread
/// does. All times are on the overlay clock. An unreadable, malformed or
/// unsupported status is the shipped "unavailable" state, which presents as
/// blocked without a cooldown.
pub fn project(
    status: Option<&Value>,
    devices: Option<&Value>,
    requested_ms: u64,
    completed_ms: u64,
    reread: bool,
) -> Projection {
    let show = |state, until_ms| Projection::Show(SmartAuto { state, until_ms });
    let unavailable = || Projection::Show(SmartAuto::unavailable());
    // `ms` of service-reported time left, counted from `from`.
    let until = |from: u64, ms: u64| {
        let until = from.saturating_add(ms);
        (ms > 0 && until > completed_ms).then_some(until)
    };
    let Some(status) = status else {
        return unavailable();
    };
    match status["state"].as_str() {
        Some("ready") => {
            let valid = &status["validForMs"];
            let verified = matches!(
                status["reason"].as_str(),
                Some("current_verified" | "previous_verified_rollback")
            );
            let (Some(id), Some(reason)) = (
                bounded(&status["deviceId"], 4096),
                status["reason"].as_str().and_then(reason),
            ) else {
                return unavailable();
            };
            let state = SmartAutoState::Ready {
                device: device_name(devices, id, "verified microphone"),
                reason,
            };
            match valid.as_u64() {
                Some(ms) if ms <= 120_000 => match until(requested_ms, ms) {
                    Some(at) => show(state, Some(at)),
                    None => Projection::Stale,
                },
                None if valid.is_null() && !verified => show(state, None),
                _ => unavailable(),
            }
        }
        Some("probing") => {
            let remaining = status["remainingMs"].as_u64().filter(|ms| *ms <= 75_000);
            let (Some(id), Some(phase), Some(remaining)) = (
                bounded(&status["deviceId"], 4096),
                status["phase"].as_str().and_then(phase),
                remaining,
            ) else {
                return unavailable();
            };
            let state = SmartAutoState::Probing {
                device: device_name(devices, id, "included microphone"),
                phase,
            };
            if remaining == 0 && status["phase"] == "stopping" {
                // The probe is finishing and gives no further deadline.
                return show(state, None);
            }
            match until(requested_ms, remaining) {
                Some(at) => show(state, Some(at)),
                None => Projection::Stale,
            }
        }
        Some("blocked") => {
            let retry = &status["retryAfterMs"];
            if bounded(&status["message"], 1024).is_none() {
                return unavailable();
            }
            if retry.is_null() {
                return show(SmartAutoState::Blocked, None);
            }
            match retry.as_u64().filter(|ms| *ms <= 900_000) {
                Some(ms) => match until(if reread { completed_ms } else { requested_ms }, ms) {
                    Some(at) => show(SmartAutoState::Blocked, Some(at)),
                    None => Projection::Stale,
                },
                None => unavailable(),
            }
        }
        _ => unavailable(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inventory() -> Value {
        json!({"devices":[{"id":"synthetic-built-in","name":"Synthetic Microphone"}]})
    }
    fn shown(p: Projection) -> SmartAuto {
        match p {
            Projection::Show(s) => s,
            Projection::Stale => panic!("stale"),
        }
    }
    fn at(status: &Value, devices: Option<&Value>, requested: u64, completed: u64) -> Projection {
        project(Some(status), devices, requested, completed, false)
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
    fn ready_probing_and_blocked_keep_their_state_and_a_deadline_from_the_request_time() {
        let devices = inventory();
        let ready = json!({"state":"ready","deviceId":"synthetic-built-in","reason":"preferred_approved","validForMs":30_000});
        let s = shown(at(&ready, Some(&devices), 1_000, 1_250));
        assert_eq!(
            s.title(1_250).unwrap(),
            "Smart Auto: Synthetic Microphone. your preferred included microphone."
        );
        assert_eq!(s.until_ms, Some(31_000), "250 ms of read latency is spent");
        let probing = json!({"state":"probing","deviceId":"synthetic-built-in","phase":"verifying","remainingMs":9_000});
        let s = shown(at(&probing, Some(&devices), 0, 100));
        assert_eq!(
            s.title(100).unwrap(),
            "Smart Auto checking Synthetic Microphone: checking signal"
        );
        assert_eq!(s.until_ms, Some(9_000));
        let cooldown = json!({"state":"blocked","message":"Synthetic block","retryAfterMs":45_000});
        let s = shown(at(&cooldown, None, 2_000, 2_100));
        assert_eq!(
            s.title(2_100).unwrap(),
            "Smart Auto blocked. Waiting for cooldown."
        );
        assert_eq!(s.until_ms, Some(47_000));
        let pinned = json!({"state":"blocked","message":"Synthetic block","retryAfterMs":null});
        let s = shown(at(&pinned, None, 0, 5));
        assert_eq!(s, SmartAuto::unavailable(), "no cooldown, no expiry");
    }

    #[test]
    fn a_read_whose_time_ran_out_in_flight_is_stale_not_shown() {
        let devices = inventory();
        let ready = json!({"state":"ready","deviceId":"synthetic-built-in","reason":"preferred_approved","validForMs":500});
        assert_eq!(
            shown(at(&ready, Some(&devices), 0, 499)).until_ms,
            Some(500)
        );
        assert_eq!(at(&ready, Some(&devices), 0, 500), Projection::Stale);
        assert_eq!(at(&ready, Some(&devices), 0, 9_000), Projection::Stale);
        let zero = json!({"state":"ready","deviceId":"synthetic-built-in","reason":"preferred_approved","validForMs":0});
        assert_eq!(at(&zero, Some(&devices), 0, 0), Projection::Stale);
        let probing =
            json!({"state":"probing","deviceId":"d","phase":"connecting","remainingMs":0});
        assert_eq!(at(&probing, None, 0, 0), Projection::Stale);
        let blocked = json!({"state":"blocked","message":"x","retryAfterMs":1_000});
        assert_eq!(at(&blocked, None, 0, 1_000), Projection::Stale);
        // A reread's cooldown starts when that read completed.
        let s = shown(project(Some(&blocked), None, 0, 1_000, true));
        assert_eq!(s.until_ms, Some(2_000));
        // A finishing probe with nothing left is kept and has no deadline.
        let stopping = json!({"state":"probing","deviceId":"d","phase":"stopping","remainingMs":0});
        let s = shown(at(&stopping, None, 0, 10));
        assert_eq!(s.until_ms, None);
        assert!(s.title(1_000_000).unwrap().contains("finishing"));
    }

    #[test]
    fn unknown_devices_use_generic_names_and_never_echo_ids() {
        let ready = json!({"state":"ready","deviceId":"PRIVATE-SYNTHETIC-ID","reason":"current_verified","validForMs":10_000});
        let text = shown(at(&ready, Some(&inventory()), 0, 0))
            .title(0)
            .unwrap();
        assert!(text.contains("verified microphone"));
        assert!(!text.contains("PRIVATE"));
        let probing = json!({"state":"probing","deviceId":"PRIVATE-SYNTHETIC-ID","phase":"connecting","remainingMs":1000});
        let text = shown(at(&probing, None, 0, 0)).title(0).unwrap();
        assert!(text.contains("included microphone") && text.contains("connecting"));
        assert!(!text.contains("PRIVATE"));
    }

    #[test]
    fn unreadable_or_unsupported_status_is_the_blocked_unavailable_state() {
        assert_eq!(
            project(None, None, 0, 0, false),
            Projection::Show(SmartAuto::unavailable())
        );
        for bad in [
            json!({"state":"mystery"}),
            json!({"state":"ready","deviceId":"d","reason":"nope","validForMs":1000}),
            json!({"state":"ready","deviceId":"d","reason":"current_verified","validForMs":null}),
            json!({"state":"ready","deviceId":"d","reason":"preferred_approved","validForMs":120_001}),
            json!({"state":"ready","deviceId":"","reason":"preferred_approved","validForMs":null}),
            json!({"state":"probing","deviceId":"d","phase":"sleeping","remainingMs":1}),
            json!({"state":"probing","deviceId":"d","phase":"connecting","remainingMs":75_001}),
            json!({"state":"blocked","message":"","retryAfterMs":null}),
            json!({"state":"blocked","message":"x","retryAfterMs":900_001}),
        ] {
            assert_eq!(
                at(&bad, Some(&inventory()), 0, 0),
                Projection::Show(SmartAuto::unavailable()),
                "{bad}"
            );
        }
        // A fixed verified-reason-free ready state may be unbounded.
        let open = json!({"state":"ready","deviceId":"synthetic-built-in","reason":"preferred_approved","validForMs":null});
        let s = shown(at(&open, Some(&inventory()), 0, 0));
        assert!(s.title(0).unwrap().starts_with("Smart Auto: Synthetic"));
        assert_eq!(s.until_ms, None);
    }

    #[test]
    fn only_device_naming_states_need_the_inventory() {
        assert!(needs_inventory(Some(&json!({"state":"ready"}))));
        assert!(needs_inventory(Some(&json!({"state":"probing"}))));
        assert!(!needs_inventory(Some(&json!({"state":"blocked"}))));
        assert!(!needs_inventory(None));
    }
}
