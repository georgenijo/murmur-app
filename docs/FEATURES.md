# Murmur — Feature Map

Current as of **v0.21.2**. This is the breadth-first inventory of what ships; each area links to its detailed feature doc. For system structure see [ARCHITECTURE.md](ARCHITECTURE.md).

---

## 1. Core dictation

**[docs/features/transcription.md](features/transcription.md)**

- Hold a key, speak, release — text lands on the clipboard and (optionally) pastes into the focused app.
- Fully offline. No cloud calls, no API keys, no telemetry leaving the machine.
- Seven models across three local engines, all from one catalog (see [Models](#3-models-and-runtime)).
- One final-after-stop transcription path for every backend. Delivery happens exactly once.
- Recordings under 0.3s are discarded as phantom triggers.
- Imported-file transcription (`transcribe_file`) runs the same pipeline with live-only stages skipped.

### Voice activity detection — [features/vad.md](features/vad.md)

- Silero VAD v5.1.2 trims silence before inference, preventing whisper hallucination loops.
- Sensitivity 0–100 (default 50) → threshold `1.0 - sensitivity/100`.
- No speech detected → transcription is skipped entirely.
- Context is cached per blocking worker rather than rebuilt per utterance.

### Recording modes — [features/recording-modes.md](features/recording-modes.md)

| Mode | Behavior |
|------|----------|
| Hold Down | Hold the trigger key to record; release to stop and transcribe |
| Double-Tap | Two taps (each <200ms, gap <400ms) start; single tap stops |
| Both | Both at once, via 200ms deferred hold promotion |

- Trigger keys: Left Shift, Left Option, Right Control.
- Modifier+letter combos are rejected so normal typing never triggers recording.
- Optional amber overlay flash when a double-tap's second-tap window expires (`hotkeyMissFeedback`, off by default).
- Global disable from the tray ("Disable Murmur") or the overlay's power button.

---

## 2. Text delivery

**[docs/features/text-injection.md](features/text-injection.md)**

- **Clipboard-first, always.** Auto-paste is layered on top and never the only path.
- Native `CGEvent` Cmd+V with an `osascript` fallback; configurable 10–500ms delay; one retry; timeout-bounded.
- Auto-paste failure emits a hint — the text is already on the clipboard.
- **File output** — numbered `.txt` transcripts and/or 16kHz mono `.wav` audio to a chosen folder (default `Documents/Murmur`). While file output is on, auto-paste is suppressed without overwriting the user's stored preference.

---

## 3. Models and runtime

**[docs/features/models.md](features/models.md)**

| Model | Engine | Accelerator | Size |
|-------|--------|-------------|------|
| Parakeet v3 | FluidAudio / Core ML | Apple Neural Engine | ~470 MB |
| Parakeet v2 | sherpa-onnx | CPU | ~1.2 GB |
| Whisper Tiny / Base / Small / Medium (`.en`) | whisper.cpp | Metal GPU | 75 MB – 1.5 GB |
| Whisper Large v3 Turbo | whisper.cpp | Metal GPU | ~3 GB |

- One catalog declares backend, accelerator, capabilities, install kind, and platform requirement per model. Unknown identifiers fail closed — no silent cross-model fallback.
- Serialized load / warm / readiness / unload lifecycle with generation-ordered status events.
- **Warm-on-record**: the model begins loading the moment recording starts, overlapping load with speech.
- Configurable idle release (5 min / 15 min / never).
- Streaming downloads with progress, atomic temp-then-rename publication, resumable Core ML extraction, and automatic Silero VAD co-download.

---

## 4. Text intelligence

All stages are deterministic and local. They run in one ordered pipeline: **cleanup → voice commands → smart correction → smart formatting → IDE context → CLI formatting**.

### Cleanup
Filler removal ("um", "uh") and sentence capitalization, each independently toggleable.

### Smart formatting — [features/smart-formatting.md](features/smart-formatting.md)
Turns clear spoken enumerations into lists; applies explicitly cued email/URL/symbol/quote/paragraph grammar; handles bounded same-utterance restatements ("no wait, make that…"). Bypassed in CLI/code/verbatim contexts and for imported files.

### CLI command formatting — [features/cli-command-formatting.md](features/cli-command-formatting.md)
Deterministic formatting for spoken npm/npx, Git, Cargo, Docker, kubectl and similar commands — versions, flags, paths, operators, quotes, canonical aliases. Detection is prefix/trigger/profile bounded; project `package.json` names extend the local lexicon; ordinary prose is untouched.

### Vocabulary and correction
- **Code-vocabulary scan** — breadth-first walk of a chosen project folder (caps: 1000 files / 32 MB / 512 KB per file), extracting identifiers with live progress, a searchable view-all pop-out, and a cap warning. Top 96 terms feed Whisper's token-bound prompt; top 500 feed Smart Correction.
- **Smart Correction** — post-model correction applied to *every* backend's output. Tier 1 exact map, Tier 2 phonetic "sounds-like" restricted to structured identifiers (camelCase / snake_case / digit) so ordinary English isn't rewritten.
- **Explicit spoken aliases** — [features/vocabulary-aliases.md](features/vocabulary-aliases.md). Map exact recognized variants ("Tori", "Tory") to a canonical written term ("Tauri"). Ambiguity, cycles, and command conflicts are rejected atomically.

### Voice Commands 2.0 — [features/voice-commands.md](features/voice-commands.md)
Typed, persistent local commands: text replacements and multiline snippets, deterministic `{{date}}` / `{{time}}` variables, explicitly permitted `{{clipboard}}` insertion, global and per-app scopes, conflict validation, and a no-paste preview/test UI.

### Correct and Teach — [features/correct-and-teach.md](features/correct-and-teach.md)
Edit the newest history entry and Murmur proposes **one bounded replacement** to learn, scoped global / app / project. Uses uniquely provable case-insensitive context alignment, so casing differences can't widen a one-word fix into a sentence rule; ambiguous alignments fail closed. **Teach specific term** lets you select one exact heard term inside a longer sentence when automatic extraction is too broad. Nothing persists without explicit confirmation.

### Personal knowledge store — [features/personal-knowledge-store.md](features/personal-knowledge-store.md)
Versioned local SQLite store for replacement rules, vocabulary terms, snippets, and saved transforms. Bounded search, scoped inspection, create/edit/enable/disable/delete, atomic export/import, visible recovery state, confirmed delete-all, deterministic migration with backup recovery and quarantine.

### Per-app dictation context — [features/per-app-profiles.md](features/per-app-profiles.md)
Per-bundle-ID profiles override auto-paste, cleanup, smart formatting, and CLI formatting, and select an explicit **Writing Style** (Inherit / Conversational / Polished prose / Code-technical / Verbatim / Notes). Resolved once into an immutable recording-start snapshot; app type is never inferred and app content is never captured. A bounded memory-only running-app picker with manual bundle-ID fallback fills the profile list.

### IDE context — [features/ide-context.md](features/ide-context.md)
Opt-in, per-profile, memory-only index of user-selected local roots. Corrects unique project symbols and canonicalizes explicitly triggered `@file` mentions to root-relative text. Never reads screen, selection, or clipboard. Ambiguous or stale references stay unchanged. Index contents are never persisted — only the chosen root strings are.

---

## 5. Selected-text transform

**[docs/features/selected-text-transform.md](features/selected-text-transform.md)** · ADR: [signed local-LLM sidecar](decisions/2026-07-20-signed-local-llm-sidecar.md)

Hold a dedicated key with text selected in any app, speak an instruction, review a local LLM's proposal, approve or undo.

- **Independent hotkey** (`alt_r` / `ctrl_l` / `shift_r`), rejects the dictation key. Off by default.
- **Signed local sidecar** running Qwen2.5-1.5B-Instruct Q4_K_M — size + SHA-256 pinned, spawned with empty env and the model as a read-only fd, no network, hardened runtime + App Sandbox.
- **Review-first**: a word diff in a non-focusable popover. Approve writes via AX set-value or a clipboard-restoring paste fallback; Undo restores the frozen original. It never auto-applies.
- **Presets and saved transforms**: Shorten, Bullets, Professional, Fix grammar, Casual, plus user-defined transforms. Presets shadow saved transforms with the same normalized name.
- **Selection capture** works in AX-native apps directly, and in Chromium/Electron webviews via an AX retry ladder then a sentinel-guarded synthetic Cmd+C that snapshots and restores the entire pasteboard.
- **Fail-closed**: positively detected secure fields, denied Accessibility, and *errored* secure-field checks all refuse — the clipboard fallback never runs where a password field can't be ruled out.
- **Escape cancels** during capture, listening, and thinking, carrying the exact pass ID so a delayed handler can't cancel the next pass.
- Mutually exclusive with dictation in both directions; a refused keypress flashes an amber busy indicator instead of failing silently.

---

## 6. Interface

### Main window
Status indicator, recording controls, transcription history (50 entries, click to copy, Correct-and-Teach entry point), usage stats (words, WPM, recordings, approximate tokens), permissions banner, update and about modals. Hides on close.

### Overlay — [features/overlay.md](features/overlay.md)
Notch-anchored Dynamic Island. Idle sits flush with the notch showing a small mic tab; recording expands with a red dot and a 7-bar waveform driven by real audio levels at 60fps via direct DOM writes; processing shows a spinner. Hover for 150ms opens a compact quick-settings card (intent-gated, so a graze doesn't pop it). Single click stops; double-click toggles locked mode. Non-activating — clicks never steal focus. Geometry comes entirely from Rust and re-derives on display changes.

### Settings — [reference/settings.md](reference/settings.md)
Seven task-oriented pages: **Recording**, **Transcription**, **Transform**, **Text & Vocabulary**, **Delivery**, **Performance**, **General**. Knowledge management lives under Text & Vocabulary.

### Onboarding — [features/onboarding-flow.md](features/onboarding-flow.md)
First-launch wizard: Welcome → Microphone → Accessibility → Model download → Done. The mic step fires the native macOS prompt in-app; both permission steps poll live so a grant made in System Settings flips the step on return; denied/stale-TCC states get inline reset-and-retry. Already-downloaded models are detected and badged. Existing installs with permissions and a model are grandfathered. Re-runnable from Settings → General.

### System tray
Static white waveform icon. Menu: Show Murmur, Disable Murmur (check item), Quit. Left-click shows the main window.

---

## 7. Diagnostics and evaluation

### Log viewer — [features/log-viewer.md](features/log-viewer.md)
Separate window with Events, Performance/Runs, Transforms, and Reports tabs. Stream chips (`pipeline`, `audio`, `keyboard`, `transform`, `system`) and level filters, expandable JSON rows, auto-scroll that disengages on manual scroll, copy-all and clear.

### Performance diagnostics — [features/performance-diagnostics.md](features/performance-diagnostics.md)
Bounded local run history (200 runs, 600 resource samples) in SQLite: per-stage timings, CPU/memory timelines, transform-stage correlation, warm-vs-cold state, and RSS deltas. Incomplete runs claiming success are rejected. No dictated text is ever recorded.

### Report comparison — [features/diagnostic-report-comparison.md](features/diagnostic-report-comparison.md)
Portable reports import into a session-only Reports workspace for schema-validated side-by-side comparison. Imported data is never silently adopted into local run history; invalid or oversized reports fail closed.

### Performance Lab — [features/performance-lab.md](features/performance-lab.md)
Benchmark installed models against bundled fixtures on your own machine. Three accuracy tiers per model/fixture — raw decoder WER, normalized WER (formatting differences don't count as errors), and delivered WER after the production transform pipeline. Stress fixtures (jargon, numbers, disfluent, 64s extra-long, fast speech) de-saturate ranking. Shared Metal/ANE init is measured separately rather than charged to the first model loaded. Fastest ranks by strict minimum duration-weighted realtime factor; Balanced uses normalized WER within two accuracy points, a 10% speed band, then lower memory. Reports export as self-identifying JSON with optional auto-save.

### Evaluation harness — [features/evaluation-harness.md](features/evaluation-harness.md)
Strict versioned fixtures, a deterministic no-hardware CI tier, an opt-in installed-model/audio tier, and machine-readable recognition/transformation/delivery reports via `murmur-eval`.

### Transform diagnostics
Every transform-key hold is recorded as a content-free `TransformAttemptV1` with ordered enum-only phase outcomes — including refused, cancelled, and superseded passes. **Capture next transform** is an explicit, confirmed, in-memory arm that stores one pass's real content locally (`0700`/`0600`, max 3, 7-day expiry, no export) for cases where exact content must be inspected.

---

## 8. Platform, privacy, distribution

- macOS 14+ on Apple Silicon (Core ML/ANE); Whisper and CPU Parakeet also build for Linux.
- Developer ID signed and notarized; hardened runtime; sidecar ships with split entitlements; release finalization fails closed on any unexpected bundle executable.
- **Auto-updater** — [features/auto-updater.md](features/auto-updater.md). Background check on launch and every 24h against `latest-v2.json`, ed25519-signed, min-version enforcement (no skip/dismiss when required), skip/dismiss otherwise, progress and auto-relaunch.
- **Privacy boundaries**: release `pipeline` events drop all strings; `transform` events are restricted to an explicit stable vocabulary in *all* builds; knowledge content and selected paths are excluded from logs; instructions never enter history or stats; audio and transcripts are written to disk only when the user turns file output on.
- All local data is inspectable and deletable from within the app.

---

## 9. Development surface

| Area | Location |
|------|----------|
| Rust backend | `app/src-tauri/src/` — 105 Tauri commands |
| Frontend | `app/src/` — React 18 + TypeScript + Tailwind 4 |
| LLM sidecar | `app/src-tauri/sidecars/local-llm/`, protocol in `crates/local-llm-protocol` |
| Diagnostics MCP tool | `tools/murmur-diag/` |
| Benchmark fixtures | `bench/` |
| Release/packaging scripts | `scripts/` |
| Workflow/artifact policy tests | `tests/` |

Tests: Rust unit and integration (`cargo test -- --test-threads=1`), frontend vitest (`npm test`), TypeScript (`npx tsc --noEmit`). CI runs all of these plus Omen code-quality analysis and automated PR review.
