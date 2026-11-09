//! Discovery checklist and contextual-hint state shared by every host.
//!
//! The state is the `murmur_discovery_v1` source document. Every operation is
//! pure: it takes the stored document and returns the next one only when that
//! document is a valid version 1 state and the operation changes it, so a host
//! never writes a no-op and never invents state that setup did not create.
//! Semantics follow the shipped `discovery.ts` and `useDiscovery.ts`.
use serde_json::{json, Value};

pub const SOURCE: &str = "murmur_discovery_v1";
pub const CHECKLIST: [&str; 5] = [
    "command_palette",
    "mode_binding",
    "meeting",
    "correction",
    "shortcuts",
];
pub const HINTS: [&str; 3] = ["double_tap_timing", "correct_and_teach", "browser_modes"];
/// The browsers that support exact-site modes.
const BROWSERS: [&str; 6] = [
    "com.apple.Safari",
    "com.google.Chrome",
    "com.microsoft.edgemac",
    "com.brave.Browser",
    "company.thebrowser.Browser",
    "org.chromium.Chromium",
];

/// Local proof of use, compared with the baseline taken when setup finished.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Evidence {
    pub transforms: u64,
    pub meetings: u64,
    pub corrections_taught: u64,
    pub mode_bindings: Vec<String>,
}
impl Evidence {
    /// Reads the serialized statistics and settings documents. Mode bindings are
    /// the sorted, unique `bundleId:modeId` pairs of the app profiles.
    pub fn read(statistics: &Value, settings: &Value) -> Self {
        let count = |v: &Value| v.as_u64().unwrap_or(0);
        let mut mode_bindings: Vec<String> = settings["appProfiles"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                let mode = p["modeId"].as_str().filter(|s| !s.is_empty())?;
                Some(format!("{}:{mode}", p["bundleId"].as_str()?))
            })
            .collect();
        mode_bindings.sort();
        mode_bindings.dedup();
        Self {
            transforms: count(&statistics["activity"]["transforms"]["runs"]),
            meetings: count(&statistics["activity"]["meetings"]["count"]),
            corrections_taught: count(&statistics["activity"]["corrections"]["taught"]),
            mode_bindings,
        }
    }
}

fn state(completed: Vec<&str>, evidence: &Evidence) -> Value {
    json!({
        "version": 1,
        "checklistDismissed": false,
        "completed": completed,
        "baseline": {
            "transforms": evidence.transforms,
            "meetings": evidence.meetings,
            "correctionsTaught": evidence.corrections_taught,
            "modeBindings": evidence.mode_bindings,
        },
        "pendingHints": [],
        "dismissedHints": [],
    })
}
/// A completed setup starts a new practice cycle from the evidence already present.
pub fn after_setup(evidence: &Evidence) -> Value {
    state(vec![], evidence)
}
/// Existing installs keep the proof their local usage already provides.
pub fn grandfathered(evidence: &Evidence) -> Value {
    let mut completed = vec![];
    if evidence.meetings > 0 {
        completed.push("meeting");
    }
    if evidence.corrections_taught > 0 {
        completed.push("correction");
    }
    if !evidence.mode_bindings.is_empty() {
        completed.push("mode_binding");
    }
    state(completed, evidence)
}

fn names(state: &Value, key: &str) -> Vec<String> {
    state[key]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|v| v.as_str().map(String::from))
        .collect()
}
/// Applies `change` to a copy of a valid state and returns it when it differs.
fn edit(state: &Value, change: impl FnOnce(&mut Value)) -> Option<Value> {
    if state["version"] != 1 || !state.is_object() {
        return None;
    }
    let mut next = state.clone();
    change(&mut next);
    (next != *state).then_some(next)
}
fn add(state: &mut Value, key: &str, id: &str) {
    let mut rows = names(state, key);
    if !rows.iter().any(|r| r == id) {
        rows.push(id.into());
        state[key] = json!(rows);
    }
}

/// Follows local usage. A statistics reset lowers the baseline to the new
/// counts, and anything above the baseline completes its item.
pub fn sync(state: &Value, evidence: &Evidence) -> Option<Value> {
    edit(state, |next| {
        let count = |key: &str| state["baseline"][key].as_u64().unwrap_or(0);
        let transforms = count("transforms").min(evidence.transforms);
        let meetings = count("meetings").min(evidence.meetings);
        let taught = count("correctionsTaught").min(evidence.corrections_taught);
        let known = names(&state["baseline"], "modeBindings");
        if evidence.meetings > meetings {
            add(next, "completed", "meeting");
        }
        if evidence.corrections_taught > taught {
            add(next, "completed", "correction");
        }
        if evidence.mode_bindings.iter().any(|b| !known.contains(b)) {
            add(next, "completed", "mode_binding");
        }
        next["baseline"]["transforms"] = json!(transforms);
        next["baseline"]["meetings"] = json!(meetings);
        next["baseline"]["correctionsTaught"] = json!(taught);
    })
}
pub fn complete(state: &Value, id: &str) -> Option<Value> {
    if !CHECKLIST.contains(&id) {
        return None;
    }
    edit(state, |next| add(next, "completed", id))
}
pub fn dismiss_checklist(state: &Value) -> Option<Value> {
    edit(state, |next| next["checklistDismissed"] = json!(true))
}
pub fn reopen_checklist(state: &Value) -> Option<Value> {
    edit(state, |next| next["checklistDismissed"] = json!(false))
}
pub fn enqueue_hint(state: &Value, id: &str) -> Option<Value> {
    if !HINTS.contains(&id) || names(state, "dismissedHints").iter().any(|h| h == id) {
        return None;
    }
    edit(state, |next| add(next, "pendingHints", id))
}
pub fn dismiss_hint(state: &Value, id: &str) -> Option<Value> {
    if !HINTS.contains(&id) {
        return None;
    }
    edit(state, |next| {
        let pending: Vec<String> = names(next, "pendingHints")
            .into_iter()
            .filter(|h| h != id)
            .collect();
        next["pendingHints"] = json!(pending);
        add(next, "dismissedHints", id);
    })
}

/// The hint an application event teaches, if any. Only the rejection reason
/// and the focused app's bundle id are read; transcript text is never kept.
pub fn hint_for_event(event: &str, payload: &str) -> Option<&'static str> {
    match event {
        "hotkey-tap-rejected" => {
            let value: Value = serde_json::from_str(payload).ok()?;
            (value["reason"] == "second_tap_expired").then_some("double_tap_timing")
        }
        "transcription-complete" => {
            let value: Value = serde_json::from_str(payload).ok()?;
            let bundle = value["teachingContext"]["appBundleId"].as_str()?;
            BROWSERS.contains(&bundle).then_some("browser_modes")
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::normalize;

    fn evidence(meetings: u64, taught: u64, bindings: &[&str]) -> Evidence {
        Evidence {
            transforms: 4,
            meetings,
            corrections_taught: taught,
            mode_bindings: bindings.iter().map(|b| b.to_string()).collect(),
        }
    }
    fn fresh() -> Value {
        after_setup(&evidence(2, 1, &["com.example.Notes:builtin.email"]))
    }

    #[test]
    fn documents_are_accepted_by_the_source_normalizer() {
        for doc in [fresh(), grandfathered(&evidence(2, 1, &["a:b"]))] {
            assert_eq!(normalize(SOURCE, doc.clone()).unwrap(), doc);
        }
    }
    #[test]
    fn evidence_reads_statistics_and_sorted_unique_bindings() {
        let stats = json!({"activity":{"transforms":{"runs":7},"meetings":{"count":3},"corrections":{"taught":2}}});
        let settings = json!({"appProfiles":[
            {"bundleId":"b.app","modeId":"m"},
            {"bundleId":"a.app","modeId":"m"},
            {"bundleId":"b.app","modeId":"m"},
            {"bundleId":"c.app","modeId":""},
            {"bundleId":"d.app"}
        ]});
        let e = Evidence::read(&stats, &settings);
        assert_eq!((e.transforms, e.meetings, e.corrections_taught), (7, 3, 2));
        assert_eq!(e.mode_bindings, ["a.app:m", "b.app:m"]);
        assert_eq!(Evidence::read(&json!({}), &json!({})), Evidence::default());
    }
    #[test]
    fn setup_starts_a_new_cycle_and_existing_installs_keep_their_proof() {
        let e = evidence(2, 1, &["a:b"]);
        assert_eq!(after_setup(&e)["completed"], json!([]));
        assert_eq!(after_setup(&e)["baseline"]["meetings"], 2);
        assert_eq!(
            grandfathered(&e)["completed"],
            json!(["meeting", "correction", "mode_binding"])
        );
        assert_eq!(grandfathered(&evidence(0, 0, &[]))["completed"], json!([]));
    }
    #[test]
    fn usage_beyond_the_baseline_completes_items_once() {
        let state = fresh();
        assert!(sync(
            &state,
            &evidence(2, 1, &["com.example.Notes:builtin.email"])
        )
        .is_none());
        let used = sync(
            &state,
            &evidence(3, 2, &["com.example.Notes:builtin.email", "z.app:m"]),
        )
        .unwrap();
        assert_eq!(
            used["completed"],
            json!(["meeting", "correction", "mode_binding"])
        );
        assert!(sync(&used, &evidence(3, 2, &["z.app:m"])).is_none());
        assert_eq!(normalize(SOURCE, used.clone()).unwrap(), used);
    }
    #[test]
    fn a_statistics_reset_lowers_the_baseline_without_completing() {
        let state = fresh();
        let reset = sync(
            &state,
            &evidence(0, 0, &["com.example.Notes:builtin.email"]),
        )
        .unwrap();
        assert_eq!(reset["completed"], json!([]));
        assert_eq!(reset["baseline"]["meetings"], 0);
        assert_eq!(reset["baseline"]["correctionsTaught"], 0);
        // The next meeting now counts against the lowered baseline.
        let next = sync(
            &reset,
            &evidence(1, 0, &["com.example.Notes:builtin.email"]),
        )
        .unwrap();
        assert_eq!(next["completed"], json!(["meeting"]));
    }
    #[test]
    fn missing_or_foreign_state_is_never_created_or_changed() {
        for state in [Value::Null, json!({}), json!({"version":2}), json!([])] {
            assert!(sync(&state, &evidence(9, 9, &["a:b"])).is_none());
            assert!(complete(&state, "shortcuts").is_none());
            assert!(dismiss_checklist(&state).is_none());
            assert!(reopen_checklist(&state).is_none());
            assert!(enqueue_hint(&state, "browser_modes").is_none());
            assert!(dismiss_hint(&state, "browser_modes").is_none());
        }
    }
    #[test]
    fn completing_is_idempotent_and_rejects_unknown_items() {
        let done = complete(&fresh(), "command_palette").unwrap();
        assert_eq!(done["completed"], json!(["command_palette"]));
        assert!(complete(&done, "command_palette").is_none());
        assert!(complete(&fresh(), "nonsense").is_none());
    }
    #[test]
    fn reopening_clears_the_dismissal() {
        let hidden = dismiss_checklist(&fresh()).unwrap();
        assert_eq!(hidden["checklistDismissed"], true);
        assert!(dismiss_checklist(&hidden).is_none());
        let shown = reopen_checklist(&hidden).unwrap();
        assert_eq!(shown["checklistDismissed"], false);
        assert!(reopen_checklist(&shown).is_none());
    }
    #[test]
    fn hints_queue_once_and_never_return_after_dismissal() {
        let queued = enqueue_hint(&fresh(), "correct_and_teach").unwrap();
        assert_eq!(queued["pendingHints"], json!(["correct_and_teach"]));
        assert!(enqueue_hint(&queued, "correct_and_teach").is_none());
        let two = enqueue_hint(&queued, "browser_modes").unwrap();
        assert_eq!(
            two["pendingHints"],
            json!(["correct_and_teach", "browser_modes"])
        );
        let dismissed = dismiss_hint(&two, "correct_and_teach").unwrap();
        assert_eq!(dismissed["pendingHints"], json!(["browser_modes"]));
        assert_eq!(dismissed["dismissedHints"], json!(["correct_and_teach"]));
        assert!(enqueue_hint(&dismissed, "correct_and_teach").is_none());
        assert!(enqueue_hint(&fresh(), "nonsense").is_none());
        assert_eq!(normalize(SOURCE, dismissed.clone()).unwrap(), dismissed);
    }
    #[test]
    fn events_teach_only_the_matching_hint() {
        assert_eq!(
            hint_for_event(
                "hotkey-tap-rejected",
                r#"{"reason":"second_tap_expired","mode":"double_tap"}"#
            ),
            Some("double_tap_timing")
        );
        for reason in ["held_too_long", "combo_cancelled", "processing_skipped"] {
            assert_eq!(
                hint_for_event("hotkey-tap-rejected", &json!({"reason":reason}).to_string()),
                None
            );
        }
        let complete = |bundle: Value| {
            json!({"recordingId":1,"text":"synthetic","teachingContext":{"appBundleId":bundle}})
                .to_string()
        };
        for browser in BROWSERS {
            assert_eq!(
                hint_for_event("transcription-complete", &complete(json!(browser))),
                Some("browser_modes")
            );
        }
        for other in [json!("com.apple.Notes"), json!(null)] {
            assert_eq!(
                hint_for_event("transcription-complete", &complete(other)),
                None
            );
        }
        assert_eq!(
            hint_for_event("transcription-complete", r#"{"teachingContext":null}"#),
            None
        );
        assert_eq!(hint_for_event("transcription-complete", "not json"), None);
        assert_eq!(hint_for_event("history-changed", "{}"), None);
    }
}
