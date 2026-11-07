//! Process-local renderer write ordering. Call on the native source worker so
//! the generation fence and the publication share one serial owner.
use std::collections::HashMap;

#[derive(Debug, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SourceWrite<T> {
    Applied { value: T },
    Superseded,
}

#[derive(Default)]
pub struct SourceOrder {
    generations: HashMap<(String, String, String), u64>,
    sessions: HashMap<String, String>,
    retired: std::collections::HashSet<(String, String)>,
}
impl SourceOrder {
    /// Registration order establishes succession only within one window label.
    /// A retired renderer can never register again, including after an IPC retry.
    pub fn register(&mut self, owner: &str, session: &str) -> bool {
        if self.retired.contains(&(owner.into(), session.into())) {
            return false;
        }
        if let Some(previous) = self.sessions.insert(owner.into(), session.into()) {
            if previous != session {
                self.retired.insert((owner.into(), previous.clone()));
                self.generations
                    .retain(|(label, id, _), _| label != owner || id != &previous);
            }
        }
        true
    }

    pub fn is_current(&self, owner: &str, session: &str) -> bool {
        self.sessions
            .get(owner)
            .is_some_and(|current| current == session)
    }

    /// Separate keys may be used for snapshots and individual field intents.
    /// Failed publications fence too: the disk owner retains their exact payload.
    pub fn take(&mut self, owner: &str, session: &str, key: &str, generation: u64) -> bool {
        if !self.is_current(owner, session) {
            return false;
        }
        let last = self
            .generations
            .entry((owner.into(), session.into(), key.into()))
            .or_default();
        if generation <= *last {
            return false;
        }
        *last = generation;
        true
    }

    pub fn apply<T, E>(
        &mut self,
        owner: &str,
        session: &str,
        key: &str,
        generation: u64,
        publish: impl FnOnce() -> Result<T, E>,
    ) -> Result<SourceWrite<T>, E> {
        if !self.take(owner, session, key, generation) {
            return Ok(SourceWrite::Superseded);
        }
        publish().map(|value| SourceWrite::Applied { value })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_order_arrival_cannot_publish_older_content() {
        let root = tempfile::tempdir().unwrap();
        for key in [
            "dictation-settings",
            "murmur-theme-library",
            "usage-dashboard-collapsed",
        ] {
            let mut order = SourceOrder::default();
            order.register("main", "session-a");
            let path = root.path().join(key);
            assert_eq!(
                order
                    .apply("main", "session-a", key, 2, || std::fs::write(
                        &path, b"newer"
                    ))
                    .unwrap(),
                SourceWrite::Applied { value: () }
            );
            assert_eq!(
                order
                    .apply("main", "session-a", key, 1, || std::fs::write(
                        &path, b"older"
                    ))
                    .unwrap(),
                SourceWrite::Superseded
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"newer");
            // A reload retires its predecessor across every key.
            order.register("main", "session-b");
            assert_eq!(
                order
                    .apply("main", "session-b", key, 1, || std::fs::write(
                        &path, b"reload"
                    ))
                    .unwrap(),
                SourceWrite::Applied { value: () }
            );
        }
    }

    #[test]
    fn failures_fence_older_writes_but_allow_fresh_retries_and_other_owners() {
        let mut order = SourceOrder::default();
        order.register("main", "session");
        order.register("other", "session");
        assert_eq!(
            order.apply("main", "session", "settings", 2, || Err::<(), _>(
                "synthetic"
            )),
            Err("synthetic")
        );
        for generation in [1, 2] {
            assert_eq!(
                order
                    .apply(
                        "main",
                        "session",
                        "settings",
                        generation,
                        || -> Result<(), &str> { panic!("stale publication ran") }
                    )
                    .unwrap(),
                SourceWrite::Superseded
            );
        }
        for (owner, key, generation) in [
            ("main", "settings", 3),
            ("other", "settings", 1),
            ("main", "theme", 1),
        ] {
            assert_eq!(
                order
                    .apply(owner, "session", key, generation, || Ok::<_, &str>(()))
                    .unwrap(),
                SourceWrite::Applied { value: () }
            );
        }
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;
    #[test]
    fn reload_retires_every_key_but_concurrent_labels_remain_live() {
        let mut order = SourceOrder::default();
        assert!(order.register("main", "main-old"));
        assert!(order.register("overlay", "overlay-live"));
        assert!(order.take("main", "main-old", "settings", 100));
        assert!(order.register("main", "main-new"));
        assert!(order.take("main", "main-new", "settings", 1));
        assert!(!order.take("main", "main-old", "settings", 101));
        assert!(!order.take("main", "main-old", "never-written-key", 102));
        assert!(!order.register("main", "main-old"));
        assert!(order.take("overlay", "overlay-live", "settings", 1));
        assert!(order.take("main", "main-new", "settings", 2));
        assert!(order.take("overlay", "overlay-live", "settings", 2));
    }
    #[test]
    fn field_generations_are_independent_of_snapshot_and_other_field_fences() {
        let mut order = SourceOrder::default();
        order.register("main", "session");
        assert!(order.take("main", "session", "settings", 2));
        assert!(!order.take("main", "session", "settings", 1));
        assert!(order.take("main", "session", "settings:autoPaste", 1));
        assert!(order.take("main", "session", "settings:disabled", 2));
        assert!(!order.take("main", "session", "settings:autoPaste", 1));
        assert!(order.take("main", "session", "settings:autoPaste", 3));
    }
}
