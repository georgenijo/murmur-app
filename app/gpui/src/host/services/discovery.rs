//! Production writer and observer for the discovery checklist and hints.
//! Every decision is `murmur_core::discovery`; this module reads the owners
//! through [`Owner`], applies a decision with a compare-and-set source write and
//! retries a lost race. `Backend` is the production owner (`discovery_owner`);
//! the unit tests drive this same code with an in-memory owner.
//! Without an existing state document nothing is written, as in the shipped app.
use murmur_core::discovery as core;
use serde_json::Value;
use std::{
    future::Future,
    sync::{
        Mutex,
        atomic::{AtomicU32, Ordering},
    },
};

pub const ATTEMPTS: usize = 3;
const UNAVAILABLE: &str = "Discovery unavailable";
const CONFLICT: &str = "Discovery changed concurrently";

/// What the observer remembers between events.
#[derive(Default)]
pub struct Seen {
    primed: bool,
    /// Newest history id already seen, so only a new entry teaches.
    newest: Value,
    /// Mode bindings at the last settings event, so a newly checked one counts.
    bindings: Vec<String>,
}
/// One-shot failures for native acceptance. Nothing but the `native-host-qa`
/// scenario flag (or a unit test) can arm them, so production never fails.
#[derive(Default)]
pub struct Faults {
    read: AtomicU32,
    write: AtomicU32,
}
impl Faults {
    fn take(counter: &AtomicU32) -> bool {
        counter
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
            .is_ok()
    }
    /// Arms a named scenario; false for a name that is not one.
    #[cfg(any(test, feature = "native-host-qa"))]
    pub fn arm(&self, scenario: &str) -> bool {
        match scenario {
            "clean" => {}
            "read-fails-once" => self.read.store(1, Ordering::SeqCst),
            "write-conflict-once" => self.write.store(1, Ordering::SeqCst),
            "write-conflict-exhausts" => self.write.store(ATTEMPTS as u32, Ordering::SeqCst),
            _ => return false,
        }
        true
    }
}
#[derive(Default)]
pub struct State {
    seen: Mutex<Seen>,
    faults: Faults,
}
#[cfg(any(test, feature = "native-host-qa"))]
impl State {
    pub fn arm(&self, scenario: &str) -> bool {
        self.faults.arm(scenario)
    }
}

/// The stores discovery reads and writes.
pub trait Owner: Clone + Send + Sync + 'static {
    fn shared(&self) -> &State;
    fn booted(&self) -> bool;
    /// The stored document, `Null` when none exists.
    fn state(&self) -> impl Future<Output = Result<Value, String>> + Send;
    /// Stores `next` only while the document still equals `expected`.
    fn write(
        &self,
        expected: Value,
        next: Value,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn evidence(&self) -> impl Future<Output = Result<core::Evidence, String>> + Send;
    /// Id of the newest history entry, `Null` when history is empty.
    fn newest(&self) -> impl Future<Output = Result<Value, String>> + Send;
    fn launch(&self, work: impl Future<Output = ()> + Send + 'static);
}

/// Reads the state, applies `change` and stores the result. `None` from
/// `change` (or a missing state) means there is nothing to write.
pub async fn mutate<O: Owner>(
    o: &O,
    change: impl Fn(&Value) -> Option<Value>,
) -> Result<(), String> {
    let faults = &o.shared().faults;
    for _ in 0..ATTEMPTS {
        if Faults::take(&faults.read) {
            return Err(UNAVAILABLE.into());
        }
        let old = o.state().await?;
        if old.is_null() {
            return Ok(());
        }
        let Some(next) = change(&old) else {
            return Ok(());
        };
        if Faults::take(&faults.write) {
            continue;
        }
        if o.write(old, next).await.is_ok() {
            return Ok(());
        }
    }
    Err(CONFLICT.into())
}

/// Marks a checklist item done after the user actually used the feature.
pub fn complete<O: Owner>(o: &O, id: &'static str) {
    let o = o.clone();
    o.clone().launch(async move {
        let _ = mutate(&o, |s| core::complete(s, id)).await;
    });
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Start {
    /// An existing install keeps the proof its local usage already provides.
    Grandfather,
    /// A setup finished in this session begins a new practice cycle.
    AfterSetup,
}
/// Creates the state a profile is missing. A grandfathered start keeps any
/// state that exists; a fresh setup replaces it.
pub async fn ensure<O: Owner>(o: &O, start: Start) -> Result<(), String> {
    for _ in 0..ATTEMPTS {
        let old = o.state().await?;
        if start == Start::Grandfather && !old.is_null() {
            return Ok(());
        }
        let evidence = o.evidence().await?;
        let next = match start {
            Start::Grandfather => core::grandfathered(&evidence),
            Start::AfterSetup => core::after_setup(&evidence),
        };
        if o.write(old, next).await.is_ok() {
            return Ok(());
        }
    }
    Err(CONFLICT.into())
}
/// Native acceptance only: an isolated, freshly set up state with the timing
/// hint waiting, replacing whatever the test profile held.
#[cfg(any(test, feature = "native-host-qa"))]
pub async fn seed<O: Owner>(o: &O) -> Result<(), String> {
    for _ in 0..ATTEMPTS {
        let old = o.state().await?;
        let base = core::after_setup(&o.evidence().await?);
        let next = core::enqueue_hint(&base, "double_tap_timing").unwrap_or(base);
        if o.write(old, next).await.is_ok() {
            return Ok(());
        }
    }
    Err(CONFLICT.into())
}

/// Follows local usage: meetings, taught corrections and mode bindings.
pub async fn sync<O: Owner>(o: &O) -> Result<(), String> {
    let evidence = o.evidence().await?;
    sync_with(o, &evidence).await
}
async fn sync_with<O: Owner>(o: &O, evidence: &core::Evidence) -> Result<(), String> {
    mutate(o, |s| core::sync(s, evidence)).await
}
fn seen<O: Owner>(o: &O) -> std::sync::MutexGuard<'_, Seen> {
    o.shared().seen.lock().unwrap_or_else(|p| p.into_inner())
}
/// Called once at startup. The entry and bindings that already exist teach
/// nothing; a profile without state gets the grandfathered one when `start` says so.
pub async fn prime<O: Owner>(o: &O, start: Option<Start>) {
    if let Some(start) = start {
        let _ = ensure(o, start).await;
    }
    let newest = o.newest().await;
    let evidence = o.evidence().await;
    {
        let mut seen = seen(o);
        if let Ok(id) = newest {
            seen.newest = id;
        }
        if let Ok(evidence) = &evidence {
            seen.bindings = evidence.mode_bindings.clone();
            seen.primed = true;
        }
    }
    let _ = sync(o).await;
}

/// Reacts to a Main event. Each branch is independent and best effort.
pub fn observe<O: Owner>(o: &O, event: &str, payload: &str) {
    if !o.booted() {
        return;
    }
    let hint = core::hint_for_event(event, payload);
    let history = event == "history-changed";
    let usage = matches!(event, "statistics-changed" | "dictation-settings-changed");
    if hint.is_none() && !history && !usage {
        return;
    }
    let o = o.clone();
    o.clone().launch(async move {
        if let Some(id) = hint {
            let _ = mutate(&o, |s| core::enqueue_hint(s, id)).await;
        }
        if history && let Ok(id) = o.newest().await {
            let fresh = {
                let mut seen = seen(&o);
                let fresh = seen.primed && !id.is_null() && seen.newest != id;
                seen.newest = id;
                fresh
            };
            if fresh {
                let _ = mutate(&o, |s| core::enqueue_hint(s, "correct_and_teach")).await;
            }
        }
        if usage && let Ok(evidence) = o.evidence().await {
            let bound = {
                let mut seen = seen(&o);
                let bound =
                    seen.primed && core::newly_bound(&seen.bindings, &evidence.mode_bindings);
                seen.bindings = evidence.mode_bindings.clone();
                bound
            };
            // Checking a binding completes the item, as the shipped
            // ModesManager does, even for a pair that was bound before.
            if bound {
                let _ = mutate(&o, |s| core::complete(s, "mode_binding")).await;
            }
            let _ = sync_with(&o, &evidence).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, atomic::AtomicBool};

    #[derive(Default)]
    struct Store {
        doc: Mutex<Value>,
        evidence: Mutex<core::Evidence>,
        newest: Mutex<Value>,
        /// A concurrent writer that lands between our read and our write.
        racer: Mutex<Option<Value>>,
        writes: AtomicU32,
        booted: AtomicBool,
        state: State,
    }
    #[derive(Clone, Default)]
    struct Fake(Arc<Store>);
    impl Owner for Fake {
        fn shared(&self) -> &State {
            &self.0.state
        }
        fn booted(&self) -> bool {
            self.0.booted.load(Ordering::SeqCst)
        }
        async fn state(&self) -> Result<Value, String> {
            Ok(self.0.doc.lock().unwrap().clone())
        }
        async fn write(&self, expected: Value, next: Value) -> Result<(), String> {
            let mut doc = self.0.doc.lock().unwrap();
            if let Some(other) = self.0.racer.lock().unwrap().take() {
                *doc = other;
            }
            if *doc != expected {
                return Err("changed".into());
            }
            *doc = next;
            self.0.writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn evidence(&self) -> Result<core::Evidence, String> {
            Ok(self.0.evidence.lock().unwrap().clone())
        }
        async fn newest(&self) -> Result<Value, String> {
            Ok(self.0.newest.lock().unwrap().clone())
        }
        fn launch(&self, work: impl Future<Output = ()> + Send + 'static) {
            futures::executor::block_on(work);
        }
    }
    fn run<T>(work: impl Future<Output = T>) -> T {
        futures::executor::block_on(work)
    }
    fn evidence(meetings: u64, taught: u64, bindings: &[&str]) -> core::Evidence {
        core::Evidence {
            transforms: 0,
            meetings,
            corrections_taught: taught,
            mode_bindings: bindings.iter().map(|b| b.to_string()).collect(),
        }
    }
    fn profile(doc: Value, ev: core::Evidence) -> Fake {
        let fake = Fake::default();
        *fake.0.doc.lock().unwrap() = doc;
        *fake.0.evidence.lock().unwrap() = ev;
        fake.0.booted.store(true, Ordering::SeqCst);
        fake
    }
    fn doc(fake: &Fake) -> Value {
        fake.0.doc.lock().unwrap().clone()
    }
    fn set(fake: &Fake, ev: core::Evidence) {
        *fake.0.evidence.lock().unwrap() = ev;
    }
    fn fresh() -> Value {
        core::after_setup(&evidence(0, 0, &[]))
    }

    #[test]
    fn a_missing_state_is_never_created_by_a_change() {
        let fake = profile(Value::Null, evidence(0, 0, &[]));
        assert!(run(mutate(&fake, core::dismiss_checklist)).is_ok());
        complete(&fake, "shortcuts");
        assert_eq!(doc(&fake), Value::Null);
        assert_eq!(fake.0.writes.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn a_lost_race_retries_against_the_winning_state() {
        let fake = profile(fresh(), evidence(0, 0, &[]));
        let winner = core::enqueue_hint(&fresh(), "browser_modes").unwrap();
        *fake.0.racer.lock().unwrap() = Some(winner);
        run(mutate(&fake, |s| core::complete(s, "shortcuts"))).unwrap();
        let stored = doc(&fake);
        assert_eq!(stored["completed"], json!(["shortcuts"]));
        // The concurrent writer's hint survives.
        assert_eq!(stored["pendingHints"], json!(["browser_modes"]));
    }
    #[test]
    fn a_source_read_failure_changes_nothing_and_the_next_try_works() {
        let fake = profile(fresh(), evidence(0, 0, &[]));
        assert!(fake.shared().faults.arm("read-fails-once"));
        assert_eq!(
            run(mutate(&fake, core::dismiss_checklist)),
            Err(UNAVAILABLE.into())
        );
        assert_eq!(doc(&fake), fresh());
        run(mutate(&fake, core::dismiss_checklist)).unwrap();
        assert_eq!(doc(&fake)["checklistDismissed"], true);
    }
    #[test]
    fn one_write_conflict_is_retried_and_exhaustion_reports_failure() {
        let fake = profile(fresh(), evidence(0, 0, &[]));
        assert!(fake.shared().faults.arm("write-conflict-once"));
        run(mutate(&fake, core::dismiss_checklist)).unwrap();
        assert_eq!(doc(&fake)["checklistDismissed"], true);

        let fake = profile(fresh(), evidence(0, 0, &[]));
        assert!(fake.shared().faults.arm("write-conflict-exhausts"));
        assert_eq!(
            run(mutate(&fake, core::dismiss_checklist)),
            Err(CONFLICT.into())
        );
        assert_eq!(doc(&fake), fresh());
        assert_eq!(fake.0.writes.load(Ordering::SeqCst), 0);
        run(mutate(&fake, core::dismiss_checklist)).unwrap();
        assert_eq!(doc(&fake)["checklistDismissed"], true);
        assert!(!fake.shared().faults.arm("not-a-scenario"));
    }
    #[test]
    fn a_completed_profile_without_state_is_grandfathered_once() {
        let fake = profile(Value::Null, evidence(2, 1, &["a.app:m"]));
        run(ensure(&fake, Start::Grandfather)).unwrap();
        assert_eq!(
            doc(&fake)["completed"],
            json!(["meeting", "correction", "mode_binding"])
        );
        // An existing state, even a customized one, is kept.
        let kept = core::dismiss_checklist(&doc(&fake)).unwrap();
        *fake.0.doc.lock().unwrap() = kept.clone();
        run(ensure(&fake, Start::Grandfather)).unwrap();
        assert_eq!(doc(&fake), kept);
        // Setup finished in this session starts a new cycle over it.
        run(ensure(&fake, Start::AfterSetup)).unwrap();
        assert_eq!(doc(&fake)["completed"], json!([]));
    }
    #[test]
    fn priming_upgrades_a_profile_and_then_the_hints_and_checklist_are_reachable() {
        let fake = profile(Value::Null, evidence(1, 0, &[]));
        run(prime(&fake, Some(Start::Grandfather)));
        assert_eq!(doc(&fake)["completed"], json!(["meeting"]));
        observe(
            &fake,
            "hotkey-tap-rejected",
            r#"{"reason":"second_tap_expired"}"#,
        );
        assert_eq!(doc(&fake)["pendingHints"], json!(["double_tap_timing"]));
        complete(&fake, "command_palette");
        assert_eq!(
            doc(&fake)["completed"],
            json!(["meeting", "command_palette"])
        );
        run(mutate(&fake, core::dismiss_checklist)).unwrap();
        run(mutate(&fake, core::reopen_checklist)).unwrap();
        assert_eq!(doc(&fake)["checklistDismissed"], false);
    }
    #[test]
    fn priming_without_a_start_leaves_a_missing_state_alone() {
        let fake = profile(Value::Null, evidence(1, 0, &[]));
        run(prime(&fake, None));
        assert_eq!(doc(&fake), Value::Null);
    }
    #[test]
    fn events_before_the_owner_is_booted_do_nothing() {
        let fake = profile(fresh(), evidence(0, 0, &[]));
        fake.0.booted.store(false, Ordering::SeqCst);
        observe(
            &fake,
            "hotkey-tap-rejected",
            r#"{"reason":"second_tap_expired"}"#,
        );
        assert_eq!(doc(&fake), fresh());
    }
    #[test]
    fn only_a_new_history_entry_teaches() {
        let fake = profile(fresh(), evidence(0, 0, &[]));
        *fake.0.newest.lock().unwrap() = json!("a");
        run(prime(&fake, None));
        observe(&fake, "history-changed", "{}");
        assert_eq!(doc(&fake)["pendingHints"], json!([]));
        *fake.0.newest.lock().unwrap() = json!("b");
        observe(&fake, "history-changed", "{}");
        assert_eq!(doc(&fake)["pendingHints"], json!(["correct_and_teach"]));
        // A dismissed hint never returns.
        let dismissed = core::dismiss_hint(&doc(&fake), "correct_and_teach").unwrap();
        *fake.0.doc.lock().unwrap() = dismissed;
        *fake.0.newest.lock().unwrap() = json!("c");
        observe(&fake, "history-changed", "{}");
        assert_eq!(doc(&fake)["pendingHints"], json!([]));
    }
    #[test]
    fn usage_follows_statistics_and_settings_events() {
        let fake = profile(fresh(), evidence(0, 0, &[]));
        run(prime(&fake, None));
        set(&fake, evidence(1, 1, &[]));
        observe(&fake, "statistics-changed", "{}");
        assert_eq!(doc(&fake)["completed"], json!(["meeting", "correction"]));
    }
    #[test]
    fn a_checked_binding_completes_even_when_that_pair_was_bound_before() {
        let before = evidence(0, 0, &["a.app:m"]);
        let fake = profile(core::after_setup(&before), before.clone());
        run(prime(&fake, None));
        // Unbinding completes nothing.
        set(&fake, evidence(0, 0, &[]));
        observe(&fake, "dictation-settings-changed", "{}");
        assert_eq!(doc(&fake)["completed"], json!([]));
        // Checking it again does, although it is inside the setup baseline.
        set(&fake, before);
        observe(&fake, "dictation-settings-changed", "{}");
        assert_eq!(doc(&fake)["completed"], json!(["mode_binding"]));
    }
    #[test]
    fn unrelated_settings_changes_do_not_complete_a_binding() {
        let before = evidence(0, 0, &["a.app:m"]);
        let fake = profile(core::after_setup(&before), before.clone());
        run(prime(&fake, None));
        observe(&fake, "dictation-settings-changed", "{}");
        assert_eq!(doc(&fake)["completed"], json!([]));
    }
    #[test]
    fn the_scenario_seed_replaces_the_profile_state_with_a_waiting_hint() {
        let fake = profile(
            core::dismiss_checklist(&fresh()).unwrap(),
            evidence(3, 0, &[]),
        );
        run(seed(&fake)).unwrap();
        let stored = doc(&fake);
        assert_eq!(stored["checklistDismissed"], false);
        assert_eq!(stored["pendingHints"], json!(["double_tap_timing"]));
        assert_eq!(stored["baseline"]["meetings"], 3);
    }
}
