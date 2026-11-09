use crate::correct_and_teach::{
    CorrectionProposalOutcome, CorrectionProposalRequest, SpecificCorrectionProposalRequest,
};
use crate::knowledge_store::{KnowledgeEntry, KnowledgeScope};
use crate::{MutexExt, State};

pub fn propose_learned_correction(
    app: crate::host::Context,
    request: CorrectionProposalRequest,
    state: crate::host::StateRef<'_, State>,
) -> CorrectionProposalOutcome {
    let Ok(_producer) = crate::shutdown::WriterPermit::acquire() else {
        return CorrectionProposalOutcome::Unsafe {
            reason: "Murmur is shutting down".into(),
        };
    };
    let original_chars = request.original_text.chars().count() as u64;
    let corrected_chars = request.corrected_text.chars().count() as u64;
    let bundle_id = request
        .teaching_context
        .as_ref()
        .and_then(|context| context.app_bundle_id.as_deref());
    let knowledge_voice_command_phrases = match state
        .knowledge
        .voice_commands_for_context(bundle_id)
    {
        Ok(entries) => entries
            .into_iter()
            .map(|entry| entry.payload.storage_parts().0)
            .collect::<Vec<_>>(),
        Err(error) => {
            tracing::warn!(
                target: "system",
                error,
                "correct_and_teach voice command conflict check unavailable"
            );
            return CorrectionProposalOutcome::Unsafe {
                reason: "Personal knowledge is unavailable, so Murmur cannot safely review a reusable rule right now."
                    .to_string(),
            };
        }
    };
    let dictation = state.app_state.dictation.lock_or_recover();
    let outcome =
        state
            .correct_and_teach
            .propose(request, &dictation, &knowledge_voice_command_phrases);
    tracing::info!(
        target: "pipeline",
        original_chars,
        corrected_chars,
        proposal_safe = matches!(outcome, CorrectionProposalOutcome::Proposal { .. }),
        "correct_and_teach_proposal"
    );
    drop(dictation);
    if let CorrectionProposalOutcome::Proposal { proposal_id, .. } = &outcome {
        crate::completion::produce(&app, |producer| producer.correction_proposed(*proposal_id));
    }
    outcome
}

pub fn propose_specific_learned_correction(
    app: crate::host::Context,
    request: SpecificCorrectionProposalRequest,
    state: crate::host::StateRef<'_, State>,
) -> CorrectionProposalOutcome {
    let Ok(_producer) = crate::shutdown::WriterPermit::acquire() else {
        return CorrectionProposalOutcome::Unsafe {
            reason: "Murmur is shutting down".into(),
        };
    };
    let original_chars = request.original_text.chars().count() as u64;
    let source_chars = request.source.chars().count() as u64;
    let replacement_chars = request.replacement.chars().count() as u64;
    let bundle_id = request
        .teaching_context
        .as_ref()
        .and_then(|context| context.app_bundle_id.as_deref());
    let knowledge_voice_command_phrases = match state
        .knowledge
        .voice_commands_for_context(bundle_id)
    {
        Ok(entries) => entries
            .into_iter()
            .map(|entry| entry.payload.storage_parts().0)
            .collect::<Vec<_>>(),
        Err(error) => {
            tracing::warn!(
                target: "system",
                error,
                "correct_and_teach voice command conflict check unavailable"
            );
            return CorrectionProposalOutcome::Unsafe {
                reason: "Personal knowledge is unavailable, so Murmur cannot safely review a reusable rule right now."
                    .to_string(),
            };
        }
    };
    let dictation = state.app_state.dictation.lock_or_recover();
    let outcome = state.correct_and_teach.propose_specific(
        request,
        &dictation,
        &knowledge_voice_command_phrases,
    );
    tracing::info!(
        target: "pipeline",
        original_chars,
        source_chars,
        replacement_chars,
        proposal_safe = matches!(outcome, CorrectionProposalOutcome::Proposal { .. }),
        "correct_and_teach_specific_proposal"
    );
    drop(dictation);
    if let CorrectionProposalOutcome::Proposal { proposal_id, .. } = &outcome {
        crate::completion::produce(&app, |producer| producer.correction_proposed(*proposal_id));
    }
    outcome
}

pub fn confirm_learned_correction(
    app: crate::host::Context,
    proposal_id: u64,
    scope: KnowledgeScope,
    state: crate::host::StateRef<'_, State>,
) -> Result<KnowledgeEntry, String> {
    let _producer = crate::shutdown::WriterPermit::acquire()?;
    let pending = state.correct_and_teach.confirmed(proposal_id, &scope)?;
    commit_confirmation(app, proposal_id, pending, scope, state)
}
/// Native QA only: `MURMUR_QA_TEACH_FAULT=confirm` fails every rule commit and
/// `=confirm-once` only the first, after the reviewed proposal was admitted.
/// Inert unless built with `native-host-qa` and run as the QA test bundle.
#[cfg(feature = "native-host-qa")]
fn qa_confirmation_fault(app: &crate::host::Context) -> bool {
    static SPENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if app.config().identifier != "com.localdictation.gpui-host-qa110" {
        return false;
    }
    match std::env::var("MURMUR_QA_TEACH_FAULT").as_deref() {
        Ok("confirm") => true,
        Ok("confirm-once") => !SPENT.swap(true, std::sync::atomic::Ordering::SeqCst),
        _ => false,
    }
}
pub(crate) fn commit_confirmation(
    app: crate::host::Context,
    proposal_id: u64,
    pending: crate::correct_and_teach::PendingCorrection,
    scope: KnowledgeScope,
    state: crate::host::StateRef<'_, State>,
) -> Result<KnowledgeEntry, String> {
    #[cfg(feature = "native-host-qa")]
    if qa_confirmation_fault(&app) {
        // Same recovery a failed write gets: the reviewed proposal is restored.
        state.correct_and_teach.restore(proposal_id);
        return Err("Injected confirmation failure. Murmur kept the reviewed correction, so Remember can be retried.".into());
    }
    // A failed write restores the reviewed proposal so the same dialog can
    // confirm it again; success consumes it.
    let entry = state
        .correct_and_teach
        .persist(&state.knowledge, proposal_id, pending, scope)?;
    if let Err(error) = crate::commands::knowledge::refresh_correction_rules(&state) {
        tracing::warn!(target: "system", error, "correct_and_teach matcher refresh failed");
    }
    tracing::info!(
        target: "pipeline",
        scope = entry.scope.kind(),
        provenance = ?entry.provenance,
        "correct_and_teach_confirmed"
    );
    let scope = match entry.scope.kind() {
        "app" => murmur_core::completion::Scope::App,
        "project" => murmur_core::completion::Scope::Project,
        _ => murmur_core::completion::Scope::Global,
    };
    crate::completion::produce(&app, |producer| {
        producer.correction_taught(proposal_id, scope)
    });
    Ok(entry)
}

pub fn discard_learned_correction_proposal(
    proposal_id: u64,
    state: crate::host::StateRef<'_, State>,
) {
    state.correct_and_teach.discard(proposal_id);
}
