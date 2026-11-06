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
}
impl SourceOrder {
    pub fn apply<T, E>(
        &mut self,
        owner: &str,
        session: &str,
        key: &str,
        generation: u64,
        publish: impl FnOnce() -> Result<T, E>,
    ) -> Result<SourceWrite<T>, E> {
        let last = self
            .generations
            .entry((owner.into(), session.into(), key.into()))
            .or_default();
        if generation <= *last {
            return Ok(SourceWrite::Superseded);
        }
        // Fence even failed publications: the disk owner may have retained
        // their exact value for replay. Retries must use a fresh generation.
        *last = generation;
        publish().map(|value| SourceWrite::Applied { value })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn out_of_order_arrival_cannot_publish_older_content() {
        let root = tempfile::tempdir().unwrap();
        let mut order = SourceOrder::default();
        for key in [
            "dictation-settings",
            "murmur-theme-library",
            "usage-dashboard-collapsed",
        ] {
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
            // A reload has its own namespace; its first generation can publish.
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
