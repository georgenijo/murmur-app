use crate::correct_and_teach::{CorrectionProposalOutcome, CorrectionProposalRequest};
use crate::knowledge_store::{KnowledgeEntry, KnowledgeScope};
use crate::State;

#[tauri::command]
pub fn propose_learned_correction(
    request: CorrectionProposalRequest,
    state: tauri::State<'_, State>,
) -> CorrectionProposalOutcome {
    let original_chars = request.original_text.chars().count() as u64;
    let corrected_chars = request.corrected_text.chars().count() as u64;
    let dictation = state
        .app_state
        .dictation
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let outcome = state.correct_and_teach.propose(request, &dictation);
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
    crate::commands::knowledge::refresh_correction_rules(&state)?;
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
