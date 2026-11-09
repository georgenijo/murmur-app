//! Correct & Teach review flow. History is edited only by an explicit user
//! action (Save correction only, or Remember correction after a reviewed
//! proposal), never by proposing. The state machine and the service calls live
//! here so production composition and its tests share one path; `root.rs` only
//! renders this state and forwards clicks.
use super::os::Live;
use murmur_core::history::Entry;
use serde_json::{Value, json};
use std::{future::Future, pin::Pin};

pub type Fut<T> = Pin<Box<dyn Future<Output = T> + Send>>;

pub const REVIEW_FAILED: &str =
    "The correction could not be reviewed right now. Check your edit and try again.";
pub const REMEMBER_FAILED: &str =
    "The correction could not be remembered. Go back, review it again and retry.";
pub const HISTORY_FAILED: &str = "This History entry changed or is unavailable, so the correction was not saved. Close this dialog and reopen Correct & Teach on the newest entry.";
pub const RULE_SAVED_HISTORY_FAILED: &str = "The rule was remembered, but this History entry could not be updated. Use Save correction only to retry, or close this dialog.";
pub const CANCELLED: &str = "Correct & Teach was closed before this finished.";

/// The service boundary. Production implements it on `Backend`; tests supply a
/// deterministic service behind the same trait.
pub trait Gateway: Clone + Send + Sync + 'static {
    fn call(&self, command: &'static str, args: Value) -> Fut<Result<Value, String>>;
    /// Freeze the reviewed proposal and scope, then commit the learned rule.
    fn accept(&self, proposal: u64, scope: Value) -> Result<Fut<Result<(), String>>, String>;
    /// Replace the exact History entry's text; fails when the entry changed.
    fn edit(&self, entry: Entry, text: String) -> Fut<Result<(), String>>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct ScopeOption {
    pub scope: Value,
    pub label: String,
}
impl ScopeOption {
    /// Shipped wording: `Global`, `App · bundle (label)`, `Project · bundle · root`.
    pub fn caption(&self) -> String {
        let bundle = self.scope["bundleId"].as_str().unwrap_or_default();
        match self.scope["kind"].as_str() {
            Some("app") if self.label != bundle => format!("App · {bundle} ({})", self.label),
            Some("app") => format!("App · {bundle}"),
            Some("project") => format!(
                "Project · {bundle} · {}",
                self.scope["root"].as_str().unwrap_or_default()
            ),
            _ => "Global".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    pub id: u64,
    pub source: String,
    pub replacement: String,
    pub occurrences: u64,
    pub before: String,
    pub after: String,
    pub scopes: Vec<ScopeOption>,
}
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Proposal(Proposal),
    Unsafe(String),
}
impl Outcome {
    /// Parse the service's camelCase `CorrectionProposalOutcome` wire shape.
    pub fn parse(value: &Value) -> Option<Self> {
        match value["kind"].as_str()? {
            "unsafe" => Some(Self::Unsafe(value["reason"].as_str()?.to_owned())),
            "proposal" => {
                let text = |key: &str| value[key].as_str().map(str::to_owned);
                let scopes = value["scopeOptions"]
                    .as_array()?
                    .iter()
                    .map(|option| {
                        Some(ScopeOption {
                            scope: option.get("scope").filter(|s| s.is_object())?.clone(),
                            label: option["label"].as_str()?.to_owned(),
                        })
                    })
                    .collect::<Option<Vec<_>>>()?;
                if scopes.is_empty() {
                    return None;
                }
                Some(Self::Proposal(Proposal {
                    id: value["proposalId"].as_u64()?,
                    source: text("source")?,
                    replacement: text("replacement")?,
                    occurrences: value["occurrenceCount"].as_u64()?,
                    before: text("originalText")?,
                    after: text("correctedText")?,
                    scopes,
                }))
            }
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Edit,
    AutomaticReview,
    SpecificEdit,
    SpecificReview,
    /// A rule was remembered but the History edit failed; only the edit retries.
    HistoryRetry,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Settled {
    Reviewed {
        specific: bool,
        result: Result<Outcome, String>,
    },
    /// The requested save completed; close the dialog.
    Closed,
    /// Nothing was saved. Stay on the current step and explain why.
    Failed(String),
    /// The rule is remembered; the History edit is still pending.
    RuleSaved(String),
}

pub struct Teaching {
    pub entry: Entry,
    pub epoch: u64,
    pub step: Step,
    pub outcome: Option<Outcome>,
    pub scope: usize,
    pub busy: bool,
    pub error: Option<String>,
}
impl Teaching {
    pub fn new(entry: Entry, epoch: u64) -> Self {
        Self {
            entry,
            epoch,
            step: Step::Edit,
            outcome: None,
            scope: 0,
            busy: false,
            error: None,
        }
    }
    pub fn proposal(&self) -> Option<&Proposal> {
        match &self.outcome {
            Some(Outcome::Proposal(proposal)) => Some(proposal),
            _ => None,
        }
    }
    pub fn proposal_id(&self) -> Option<u64> {
        self.proposal().map(|p| p.id)
    }
    pub fn selected_scope(&self) -> Option<&ScopeOption> {
        self.proposal().and_then(|p| p.scopes.get(self.scope))
    }
    /// Review needs a real, non-blank change to the transcript.
    pub fn can_review(&self, corrected: &str) -> bool {
        !self.busy && !corrected.trim().is_empty() && corrected != self.entry.text
    }
    pub fn can_review_specific(&self, source: &str, replacement: &str) -> bool {
        !self.busy && !source.trim().is_empty() && !replacement.trim().is_empty()
    }
    /// Leave a review step. Returns the pending proposal to discard.
    pub fn back(&mut self) -> Option<u64> {
        let next = match self.step {
            Step::AutomaticReview | Step::SpecificEdit => Step::Edit,
            Step::SpecificReview => Step::SpecificEdit,
            Step::Edit | Step::HistoryRetry => return None,
        };
        self.step = next;
        self.error = None;
        let id = self.proposal_id();
        self.outcome = None;
        id
    }
    /// Open the specific-term form. Prefills with the automatic rule when there
    /// is one. Returns `(source, replacement, proposal to discard)`.
    pub fn open_specific(&mut self) -> (String, String, Option<u64>) {
        let (source, replacement) = self
            .proposal()
            .map(|p| (p.source.clone(), p.replacement.clone()))
            .unwrap_or_default();
        let id = self.proposal_id();
        self.outcome = None;
        self.error = None;
        self.step = Step::SpecificEdit;
        (source, replacement, id)
    }
    pub fn select_scope(&mut self, index: usize) {
        if self.proposal().is_some_and(|p| index < p.scopes.len()) {
            self.scope = index;
        }
    }
    /// Apply a finished operation. Returns true when the dialog should close.
    pub fn settle(&mut self, settled: Settled) -> bool {
        self.busy = false;
        match settled {
            Settled::Closed => {
                self.outcome = None;
                return true;
            }
            Settled::Failed(message) => self.error = Some(message),
            Settled::RuleSaved(message) => {
                self.outcome = None;
                self.step = Step::HistoryRetry;
                self.error = Some(message);
            }
            Settled::Reviewed { specific, result } => match (specific, result) {
                (_, Err(message)) => self.error = Some(message),
                (true, Ok(Outcome::Unsafe(reason))) => self.error = Some(reason),
                (specific, Ok(outcome)) => {
                    self.step = if specific {
                        Step::SpecificReview
                    } else {
                        Step::AutomaticReview
                    };
                    self.scope = 0;
                    self.error = None;
                    self.outcome = Some(outcome);
                }
            },
        }
        false
    }
}

pub enum Review {
    Automatic { corrected: String },
    Specific { source: String, replacement: String },
}

/// Native QA only: force one service failure so a tester can see its message.
/// `MURMUR_QA_TEACH_FAULT=confirm` fails the rule commit; `=history` fails the
/// History edit. Inert unless the binary is built with `native-host-qa`.
fn qa_fault(kind: &str) -> bool {
    cfg!(feature = "native-host-qa")
        && std::env::var("MURMUR_QA_TEACH_FAULT").is_ok_and(|value| value == kind)
}

impl Gateway for super::backend::Backend {
    fn call(&self, command: &'static str, args: Value) -> Fut<Result<Value, String>> {
        let backend = self.clone();
        Box::pin(async move { backend.call(command, args).await })
    }
    fn accept(&self, proposal: u64, scope: Value) -> Result<Fut<Result<(), String>>, String> {
        if qa_fault("confirm") {
            return Err("Injected confirmation failure".into());
        }
        let job = self.services.accept_correction(proposal, scope)?;
        Ok(Box::pin(async move {
            job.await
                .map_err(|_| "Correction owner unavailable".to_owned())?
                .map(|_| ())
        }))
    }
    fn edit(&self, entry: Entry, text: String) -> Fut<Result<(), String>> {
        let backend = self.clone();
        Box::pin(async move {
            if qa_fault("history") {
                return Err("Injected History failure".into());
            }
            let (revision, snapshot) = backend.services.history_snapshot().await?;
            let target = entry.target();
            if !snapshot.history.contains(&target) {
                return Err("History changed".into());
            }
            backend.services.mutate_history(
                revision,
                murmur_core::completion::Command::History(
                    murmur_core::completion::HistoryMutation::Edit { target, text },
                ),
            )
        })
    }
}

pub async fn discard<G: Gateway>(gateway: &G, proposal: u64) {
    let _ = gateway
        .call(
            "discard_learned_correction_proposal",
            json!({"proposalId":proposal}),
        )
        .await;
}

/// Propose only. This never touches History. A proposal that arrives after the
/// dialog closed is discarded so it cannot be confirmed later.
pub async fn review<G: Gateway>(
    gateway: &G,
    live: &Live,
    entry: &Entry,
    review: Review,
) -> Settled {
    let context = entry.target().get("teachingContext").cloned();
    let (command, request, specific) = match review {
        Review::Automatic { corrected } => (
            "propose_learned_correction",
            json!({"originalText":entry.text,"correctedText":corrected,"teachingContext":context}),
            false,
        ),
        Review::Specific {
            source,
            replacement,
        } => (
            "propose_specific_learned_correction",
            json!({"originalText":entry.text,"source":source,"replacement":replacement,"teachingContext":context}),
            true,
        ),
    };
    let result = gateway
        .call(command, json!({"request":request}))
        .await
        .ok()
        .and_then(|value| Outcome::parse(&value))
        .ok_or_else(|| REVIEW_FAILED.to_owned());
    if !live()
        && let Ok(Outcome::Proposal(proposal)) = &result
    {
        discard(gateway, proposal.id).await;
    }
    Settled::Reviewed { specific, result }
}

/// Save correction only: edit History without storing knowledge, then discard
/// any pending capability. A failed edit keeps the review on screen.
pub async fn save_only<G: Gateway>(
    gateway: &G,
    live: &Live,
    entry: &Entry,
    corrected: String,
    proposal: Option<u64>,
) -> Settled {
    if !live() {
        return Settled::Failed(CANCELLED.into());
    }
    if gateway.edit(entry.clone(), corrected).await.is_err() {
        return Settled::Failed(HISTORY_FAILED.into());
    }
    if let Some(id) = proposal {
        discard(gateway, id).await;
    }
    Settled::Closed
}

/// Remember correction: commit the reviewed rule in the chosen scope, then
/// edit History. Nothing is saved when the commit fails; if only the edit
/// fails the dialog says so instead of claiming success.
pub async fn remember<G: Gateway>(
    gateway: &G,
    live: &Live,
    entry: &Entry,
    corrected: String,
    proposal: u64,
    scope: Value,
) -> Settled {
    if !live() {
        return Settled::Failed(CANCELLED.into());
    }
    let committed = match gateway.accept(proposal, scope) {
        Ok(job) => job.await,
        Err(error) => Err(error),
    };
    if committed.is_err() {
        return Settled::Failed(REMEMBER_FAILED.into());
    }
    match gateway.edit(entry.clone(), corrected).await {
        Ok(()) => Settled::Closed,
        Err(_) => Settled::RuleSaved(RULE_SAVED_HISTORY_FAILED.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    const BEFORE: &str = "ask synthetic neo today and synthetic neo tomorrow";
    const AFTER: &str = "ask synthetic nijo today and synthetic nijo tomorrow";

    fn entry() -> Entry {
        Entry::project(
            1,
            json!({"id":"e1","text":BEFORE,"timestamp":1,"duration":1.0,
                "teachingContext":{"appBundleId":"synthetic.app","appLabel":"Synthetic","projectRoot":null}}),
        )
        .unwrap()
    }
    fn proposal_json(id: u64) -> Value {
        json!({"kind":"proposal","proposalId":id,"source":"neo","replacement":"nijo",
            "occurrenceCount":2,"originalText":BEFORE,"correctedText":AFTER,
            "scopeOptions":[
                {"scope":{"kind":"global"},"label":"All apps"},
                {"scope":{"kind":"app","bundleId":"synthetic.app"},"label":"Synthetic"}]})
    }
    fn live() -> Live {
        Arc::new(|| true)
    }

    /// Deterministic service that records every call in order.
    #[derive(Clone, Default)]
    struct Fake {
        log: Arc<Mutex<Vec<String>>>,
        reply: Arc<Mutex<Option<Value>>>,
        fail_call: bool,
        fail_accept: bool,
        fail_edit: bool,
    }
    impl Fake {
        fn replying(value: Value) -> Self {
            let fake = Self::default();
            *fake.reply.lock().unwrap() = Some(value);
            fake
        }
        fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }
    impl Gateway for Fake {
        fn call(&self, command: &'static str, args: Value) -> Fut<Result<Value, String>> {
            self.log.lock().unwrap().push(format!("{command} {args}"));
            let result = if self.fail_call {
                Err("synthetic service failure".to_owned())
            } else {
                Ok(self.reply.lock().unwrap().clone().unwrap_or(Value::Null))
            };
            Box::pin(async move { result })
        }
        fn accept(&self, proposal: u64, scope: Value) -> Result<Fut<Result<(), String>>, String> {
            self.log
                .lock()
                .unwrap()
                .push(format!("accept {proposal} {scope}"));
            let fail = self.fail_accept;
            Ok(Box::pin(async move {
                if fail {
                    Err("changed".to_owned())
                } else {
                    Ok(())
                }
            }))
        }
        fn edit(&self, entry: Entry, text: String) -> Fut<Result<(), String>> {
            self.log
                .lock()
                .unwrap()
                .push(format!("edit {} {text}", entry.key));
            let fail = self.fail_edit;
            Box::pin(async move {
                if fail {
                    Err("History changed".to_owned())
                } else {
                    Ok(())
                }
            })
        }
    }
    fn block<T>(work: impl Future<Output = T>) -> T {
        futures::executor::block_on(work)
    }
    fn edits(log: &[String]) -> usize {
        log.iter().filter(|l| l.starts_with("edit ")).count()
    }

    #[test]
    fn wire_shape_parses_proposals_and_unsafe_outcomes_and_rejects_the_rest() {
        let Some(Outcome::Proposal(p)) = Outcome::parse(&proposal_json(7)) else {
            panic!("proposal")
        };
        assert_eq!((p.id, p.occurrences, p.scopes.len()), (7, 2, 2));
        assert_eq!(p.scopes[0].caption(), "Global");
        assert_eq!(p.scopes[1].caption(), "App · synthetic.app (Synthetic)");
        assert_eq!(
            Outcome::parse(&json!({"kind":"unsafe","reason":"Ambiguous"})),
            Some(Outcome::Unsafe("Ambiguous".into()))
        );
        let mut no_scope = proposal_json(7);
        no_scope["scopeOptions"] = json!([]);
        for bad in [Value::Null, json!({"kind":"saved"}), no_scope] {
            assert_eq!(Outcome::parse(&bad), None);
        }
    }

    #[test]
    fn proposing_never_edits_history_and_back_discards_without_a_rule() {
        let fake = Fake::replying(proposal_json(7));
        let entry = entry();
        let settled = block(review(
            &fake,
            &live(),
            &entry,
            Review::Automatic {
                corrected: AFTER.into(),
            },
        ));
        let log = fake.log();
        assert_eq!(edits(&log), 0, "{log:?}");
        assert!(log[0].starts_with("propose_learned_correction "));
        // The recording-start scope reaches the service request unchanged.
        assert!(log[0].contains("\"appBundleId\":\"synthetic.app\""));
        let mut t = Teaching::new(entry, 1);
        t.busy = true;
        assert!(!t.settle(settled));
        assert_eq!(t.step, Step::AutomaticReview);
        assert_eq!(t.proposal_id(), Some(7));
        assert_eq!(t.selected_scope().unwrap().caption(), "Global");
        assert_eq!(t.back(), Some(7));
        assert_eq!((t.step, t.outcome.is_none()), (Step::Edit, true));
        block(discard(&fake, 7));
        let log = fake.log();
        assert_eq!(edits(&log), 0);
        assert!(!log.iter().any(|l| l.starts_with("accept")));
    }

    #[test]
    fn unsafe_and_failed_reviews_explain_themselves_and_offer_specific_term() {
        let fake = Fake::replying(json!({"kind":"unsafe","reason":"Too many changes"}));
        let entry = entry();
        let mut t = Teaching::new(entry.clone(), 1);
        let settled = block(review(
            &fake,
            &live(),
            &entry,
            Review::Automatic {
                corrected: AFTER.into(),
            },
        ));
        assert!(!t.settle(settled));
        assert_eq!(t.step, Step::AutomaticReview);
        assert_eq!(t.outcome, Some(Outcome::Unsafe("Too many changes".into())));
        let (source, replacement, pending) = t.open_specific();
        assert_eq!(
            (source.as_str(), replacement.as_str(), pending),
            ("", "", None)
        );
        assert_eq!(t.step, Step::SpecificEdit);
        let broken = Fake {
            fail_call: true,
            ..Fake::default()
        };
        let settled = block(review(
            &broken,
            &live(),
            &entry,
            Review::Specific {
                source: "neo".into(),
                replacement: "nijo".into(),
            },
        ));
        t.busy = true;
        assert!(!t.settle(settled));
        assert_eq!(t.step, Step::SpecificEdit);
        assert_eq!(t.error.as_deref(), Some(REVIEW_FAILED));
        // An unparseable reply fails closed the same way.
        let garbled = Fake::replying(json!({"kind":"proposal"}));
        let settled = block(review(
            &garbled,
            &live(),
            &entry,
            Review::Automatic {
                corrected: AFTER.into(),
            },
        ));
        assert_eq!(
            settled,
            Settled::Reviewed {
                specific: false,
                result: Err(REVIEW_FAILED.into())
            }
        );
    }

    #[test]
    fn specific_term_prefills_from_the_automatic_rule_and_unsafe_stays_on_the_form() {
        let entry = entry();
        let mut t = Teaching::new(entry.clone(), 1);
        t.settle(Settled::Reviewed {
            specific: false,
            result: Ok(Outcome::parse(&proposal_json(3)).unwrap()),
        });
        assert_eq!(
            t.open_specific(),
            ("neo".to_owned(), "nijo".to_owned(), Some(3))
        );
        assert!(t.can_review_specific("neo", "nijo"));
        assert!(!t.can_review_specific(" ", "nijo"));
        let fake = Fake::replying(json!({"kind":"unsafe","reason":"Heard text must match"}));
        let settled = block(review(
            &fake,
            &live(),
            &entry,
            Review::Specific {
                source: "neo".into(),
                replacement: "nijo".into(),
            },
        ));
        let log = fake.log();
        assert!(log[0].starts_with("propose_specific_learned_correction "));
        assert!(log[0].contains("\"source\":\"neo\""));
        t.busy = true;
        assert!(!t.settle(settled));
        assert_eq!(t.step, Step::SpecificEdit);
        assert_eq!(t.error.as_deref(), Some("Heard text must match"));
        // A reviewed specific rule moves to its own review, and Back returns to the form.
        t.settle(Settled::Reviewed {
            specific: true,
            result: Ok(Outcome::parse(&proposal_json(4)).unwrap()),
        });
        assert_eq!(t.step, Step::SpecificReview);
        assert_eq!(t.back(), Some(4));
        assert_eq!(t.step, Step::SpecificEdit);
        assert_eq!(t.back(), None);
        assert_eq!(t.step, Step::Edit);
    }

    #[test]
    fn remember_commits_the_chosen_scope_then_edits_history_once() {
        let fake = Fake::default();
        let entry = entry();
        let mut t = Teaching::new(entry.clone(), 1);
        t.settle(Settled::Reviewed {
            specific: false,
            result: Ok(Outcome::parse(&proposal_json(7)).unwrap()),
        });
        t.select_scope(1);
        t.select_scope(9);
        let scope = t.selected_scope().unwrap().scope.clone();
        let settled = block(remember(&fake, &live(), &entry, AFTER.into(), 7, scope));
        assert_eq!(settled, Settled::Closed);
        let log = fake.log();
        assert_eq!(log.len(), 2, "{log:?}");
        assert!(log[0].starts_with("accept 7 ") && log[0].contains("synthetic.app"));
        assert_eq!(log[1], format!("edit 1 {AFTER}"));
        assert!(t.settle(settled));
    }

    #[test]
    fn remember_failure_leaves_history_untouched_and_keeps_the_review_open() {
        let fake = Fake {
            fail_accept: true,
            ..Fake::default()
        };
        let entry = entry();
        let mut t = Teaching::new(entry.clone(), 1);
        t.settle(Settled::Reviewed {
            specific: false,
            result: Ok(Outcome::parse(&proposal_json(7)).unwrap()),
        });
        t.busy = true;
        let settled = block(remember(
            &fake,
            &live(),
            &entry,
            AFTER.into(),
            7,
            json!({"kind":"global"}),
        ));
        assert_eq!(settled, Settled::Failed(REMEMBER_FAILED.into()));
        assert_eq!(edits(&fake.log()), 0);
        assert!(!t.settle(settled));
        assert_eq!(t.step, Step::AutomaticReview);
        assert_eq!(t.proposal_id(), Some(7));
        assert_eq!(t.error.as_deref(), Some(REMEMBER_FAILED));
    }

    #[test]
    fn remembered_rule_with_a_failed_history_edit_is_reported_and_retryable() {
        let entry = entry();
        let fake = Fake {
            fail_edit: true,
            ..Fake::default()
        };
        let mut t = Teaching::new(entry.clone(), 1);
        t.settle(Settled::Reviewed {
            specific: false,
            result: Ok(Outcome::parse(&proposal_json(7)).unwrap()),
        });
        t.busy = true;
        let settled = block(remember(
            &fake,
            &live(),
            &entry,
            AFTER.into(),
            7,
            json!({"kind":"global"}),
        ));
        assert_eq!(
            settled,
            Settled::RuleSaved(RULE_SAVED_HISTORY_FAILED.into())
        );
        assert!(!t.settle(settled));
        assert_eq!(t.step, Step::HistoryRetry);
        assert_eq!(t.error.as_deref(), Some(RULE_SAVED_HISTORY_FAILED));
        assert_eq!(t.proposal_id(), None);
        // Retrying saves only the edit, with no second rule.
        let healthy = Fake::default();
        let settled = block(save_only(&healthy, &live(), &entry, AFTER.into(), None));
        assert_eq!(settled, Settled::Closed);
        assert_eq!(healthy.log(), vec![format!("edit 1 {AFTER}")]);
    }

    #[test]
    fn save_only_edits_history_and_discards_the_pending_rule_without_remembering() {
        let fake = Fake::default();
        let entry = entry();
        let settled = block(save_only(&fake, &live(), &entry, AFTER.into(), Some(7)));
        assert_eq!(settled, Settled::Closed);
        let log = fake.log();
        assert_eq!(log[0], format!("edit 1 {AFTER}"));
        assert!(log[1].starts_with("discard_learned_correction_proposal "));
        assert!(!log.iter().any(|l| l.starts_with("accept")));
        // A failed edit keeps the proposal so the user can retry or go back.
        let stale = Fake {
            fail_edit: true,
            ..Fake::default()
        };
        let settled = block(save_only(&stale, &live(), &entry, AFTER.into(), Some(7)));
        assert_eq!(settled, Settled::Failed(HISTORY_FAILED.into()));
        assert_eq!(stale.log().len(), 1);
    }

    #[test]
    fn closing_the_dialog_discards_a_late_proposal_and_blocks_saves() {
        let gone: Live = Arc::new(|| false);
        let entry = entry();
        let fake = Fake::replying(proposal_json(9));
        block(review(
            &fake,
            &gone,
            &entry,
            Review::Automatic {
                corrected: AFTER.into(),
            },
        ));
        assert!(fake.log().last().is_some_and(|l| {
            l.starts_with("discard_learned_correction_proposal {\"proposalId\":9}")
        }));
        let fake = Fake::default();
        assert_eq!(
            block(save_only(&fake, &gone, &entry, AFTER.into(), None)),
            Settled::Failed(CANCELLED.into())
        );
        assert_eq!(
            block(remember(
                &fake,
                &gone,
                &entry,
                AFTER.into(),
                9,
                json!({"kind":"global"})
            )),
            Settled::Failed(CANCELLED.into())
        );
        assert!(fake.log().is_empty());
    }

    #[test]
    fn review_requires_a_real_change() {
        let t = Teaching::new(entry(), 1);
        assert!(!t.can_review(BEFORE));
        assert!(!t.can_review("   "));
        assert!(t.can_review(AFTER));
    }
}
