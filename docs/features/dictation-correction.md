# Correct last dictation

Correct last dictation interprets an explicit spoken correction against the latest successful dictation delivery in the current app session. It uses the existing local speech recognizer and signed local LLM sidecar. Normal dictation never enters the correction interpreter.

## Controls

The command palette includes **Correct last dictation…**. Settings → Transforms includes an off-by-default **Correct last dictation shortcut** toggle. When enabled, **⌘⇧E** starts the instruction recording and a second press finishes it. The review window also has **Done speaking**. Escape cancels the current pass.

The optional transform model must be downloaded. Starting without a previous delivery produces an explicit error. Restarting Murmur clears the previous-delivery slot; saved history is not treated as a live insertion target.

## Interpretation

The local model proposes one JSON edit with `heard` and `replacement` strings. Rust requires one exact whole-term occurrence of the heard phrase and reconstructs the corrected transcript. The existing bounded correction alignment then checks that the result contains one unambiguous replacement. Missing or repeated source phrases, malformed JSON, unchanged results, oversized edits, and multiple changes are rejected. The model cannot request application actions or write knowledge.

An explicitly introduced trailing sequence of separate ASCII letters is literal spelling. For example, **spelled T A U R I** produces **TAURI**, preserving the recognized letters and their case. Rust uses those letters even if the model proposes another spelling. Letter names, phonetic alphabets, and spelling followed by additional instructions are not parsed as literal spelling. Speech recognition can still mishear letters; the review remains necessary.

## Review and delivery

The review shows the exact reconstructed change before any write. **Copy correction** copies the proposed transcript and leaves the destination unchanged. Copying has no document Undo and does not rewrite history or statistics.

If the original app has the complete previous dictation explicitly selected when correction starts, native capture checks the selection and original application identity. A matching selection enables **Approve** and the existing transform write-back and Undo paths. Application-instance identity is checked again before replacement. Otherwise correction remains copy-only. Murmur never guesses a previous insertion range or uses a synthetic Copy gesture to acquire a correction target.

Approved corrections update the process-memory last-delivery slot, so another correction builds on the approved text. Undo restores that slot only when it still belongs to the same recording and correction. The raw and delivered history records stay unchanged. **Remember this correction…** opens a separate teaching review, including the exact phrase pair and available global, app, or project scopes from the frozen recording context. Only **Remember correction** persists a rule. Copying, approving, retrying, or cancelling does not teach automatically.

## Ownership and privacy

Correction uses the transform recording state machine, mutual-exclusion guards, cancellation token, and monotonic pass ID. Review content, approval, retry, and Undo carry the exact pass ID so stale UI actions cannot act on a later review. Content retrieval is restricted to the review webview.

Instructions and proposals do not enter dictation history, usage statistics, or structured logs. The existing explicitly consented transform diagnostic capture can include correction content. Last-delivery text and its teaching context remain in one process-memory slot.

## Verification

Rust unit tests cover literal spelling, exact reconstruction, ambiguous matches, malformed edits, and shortcut key-repeat handling. An ignored `installed_model_proposes_real_corrections` test runs the actual installed local model and production correction parser against synthetic examples. Setting `MURMUR_CORRECTION_WAV_DIR` to a directory containing `dictation.wav` and `instruction.wav` also exercises real Core ML recognition before local correction. The audio example says “Ship the release on Friday” and “Change Friday to Monday.” Frontend tests cover explicit teaching confirmation and pass-scoped review actions.

The native flow requires separate capture and review testing. A replayed PCM fixture proves application orchestration and recognition but does not prove physical microphone startup or global hotkey delivery.
