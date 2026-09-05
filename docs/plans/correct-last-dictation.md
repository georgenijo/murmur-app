# Correct last dictation, issue #676

Authorized implementation of the correction shortcut discussed with George.

- [x] Inspect existing dictation delivery, instruction ASR, local sidecar, review, and teaching boundaries.
- [x] Prove the local model can return bounded edits on representative examples.
- [x] Implement a pure correction parser/validator with literal spelling preservation.
- [x] Add an explicit correction start action and shortcut using the existing transform lifecycle.
- [x] Freeze the Rust-owned last delivery; never accept the original transcript from a frontend request.
- [x] Review the reconstructed edit, offer corrected copy, and permit native replacement only with matching explicitly selected text and existing native checks. Never select a guessed document range.
- [x] Reuse explicit teaching confirmation for an eligible bounded correction.
- [ ] Verify native flow, local-model examples, cancellation and source ownership, and full required checks.
- [ ] Review, publish PR, run exact-head performance gate, and merge when green.

The correction LLM returns one exact heard phrase and replacement. Rust rejects malformed output, absent or repeated source phrases, oversized edits, and spelling mismatches; Rust constructs the final transcript. Normal dictation never invokes this interpreter. The transform session carries a typed purpose so correction can never accidentally use selection write-back without an explicit matching selection. Copy-only approval is labeled as copy and never offers document Undo. Learning is a separate explicit action.

The original delivery stores an application identity, not an exact editable-element insertion receipt. It is insufficient evidence for automatic replacement of a guessed previous range. The first safe implementation therefore starts from the last transcript without needing selection and supports explicit selection for write-back; automatic insertion receipts can be added only with native evidence.

## Verification receipt

The installed Qwen model passes three synthetic correction examples. The optional real Core ML WAV path also passes recognition of the original dictation and correction instruction before local-model interpretation. Full Rust and frontend suites passed during implementation; final-head validation is recorded in the PR.

Native app inspection verified the command-palette action and its no-previous-dictation refusal. The review UI was inspected at 420×220 using a browser fixture. Full native capture/review remains pending: the Mac mini has no microphone, and an isolated synthetic-PCM test bundle correctly stopped at its missing microphone permission. No TCC permission was changed. The disposable capture substitute was restored to the built native helper after the attempt.

Use an authorized MacBook test build to finish microphone, shortcut, copy, matching-selection replacement, and Undo acceptance before marking the PR ready.
