use crate::correct_and_teach::{
    CorrectionProposalOutcome, CorrectionProposalRequest, SpecificCorrectionProposalRequest,
};
use crate::knowledge_store::{KnowledgeEntry, KnowledgeScope};
use crate::{MutexExt, State};

#[tauri::command]
pub fn propose_learned_correction(
    request: CorrectionProposalRequest,
    state: tauri::State<'_, State>,
) -> CorrectionProposalOutcome {
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
    outcome
}

#[tauri::command]
pub fn propose_specific_learned_correction(
    request: SpecificCorrectionProposalRequest,
    state: tauri::State<'_, State>,
) -> CorrectionProposalOutcome {
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
    outcome
}

#[tauri::command]
pub fn confirm_learned_correction(
    proposal_id: u64,
    scope: KnowledgeScope,
    state: tauri::State<'_, State>,
) -> Result<KnowledgeEntry, String> {
    let pending = state.correct_and_teach.confirmed(proposal_id, &scope)?;
    let entry =
        state
            .knowledge
            .create_learned_replacement(pending.source, pending.replacement, scope)?;
    if let Err(error) = crate::commands::knowledge::refresh_correction_rules(&state) {
        tracing::warn!(target: "system", error, "correct_and_teach matcher refresh failed");
    }
    state.correct_and_teach.discard(proposal_id);
    tracing::info!(
        target: "pipeline",
        scope = entry.scope.kind(),
        provenance = ?entry.provenance,
        "correct_and_teach_confirmed"
    );
    Ok(entry)
}

#[tauri::command]
pub fn discard_learned_correction_proposal(proposal_id: u64, state: tauri::State<'_, State>) {
    state.correct_and_teach.discard(proposal_id);
}
