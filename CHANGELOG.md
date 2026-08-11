# Changelog

All notable changes to Murmur will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/).

## [Unreleased]

### Changed

- Transcript history and usage statistics now survive WebKit storage eviction
  and manual reinstalls through Rust-owned, local-only durable files. Existing
  localStorage data migrates automatically, the 200-entry history cap remains,
  and disabling history retention still discards new transcript content (#521).

### Fixed

- Release promotion now compares draft and updater-manifest notes with the same
  deterministic whitespace normalization, avoiding formatting-only failures
  while still rejecting edited content (#512).

- Microphone startup health now retains a bounded, content-free local history
  independently of general event traffic, so idle system telemetry no longer
  resets the five-recording signal and the Performance tab no longer polls the
  full event history every two seconds (#514).

## [0.30.1] - 2026-08-09

### Fixed

- Update checks now retry transient release-feed and minimum-version policy
  failures before showing actionable connection guidance.

## [0.30.0] - 2026-08-07

### Added

- Advanced Diagnostics can pop out into a movable standalone window and now
  includes a local, content-free latency map for History, Settings pages, and
  diagnostics navigation.
- Release validation now has an opt-in, local-only personal speech corpus and
  repeatable Mac benchmark harness. The recorder, audio, and benchmark build
  remain outside consumer builds and GitHub CI.

### Changed

- History and Settings navigation now keeps isolated compositor surfaces warm,
  gates inactive diagnostics work, and renders large event histories in a
  bounded window. Typical History-to-Settings latency fell from 112 ms to
  29 ms in release-build measurements, with overall P95 reduced from 197 ms
  to 59 ms.
- Dictation delivery now validates bundled capture and model artifacts, keeps
  private-history behavior isolated per recording, and verifies that automatic
  paste still targets the application that owned the dictation.

## [0.29.0] - 2026-08-07

### Added

- Diagnostics now surface capture-startup health and can schedule a
  privacy-bounded regression watch for recurring capture problems.

### Changed

- Transform prewarming keeps the speech-recognition model warm through
  instruction capture, reducing time to a visible rewrite.

### Fixed

- Update checks and installs now have a single owner, preventing duplicate
  downloads and state replacement during install preparation. Release
  promotion can publish a validated source-controlled minimum-version policy
  and fails if release notes drift from the updater manifest (#439).
- Capture processing now clears correctly after late helper teardown, while
  blocked-recovery paths receive explicit coverage.
- The notch overlay remains attached across Stage Manager sets and Spaces.
- The log shipper no longer performs microphone enumeration while idle.

## [0.28.1] - 2026-08-07

### Fixed

- Main-window title-bar content now aligns with the native macOS traffic lights
  without crowding the recording controls (#471).
- The NotchPill caption-mirroring toggle now appears only when macOS finds the
  companion app. Removing NotchPill makes mirroring dormant and deletes the
  stale caption while preserving the preference for a later reinstall (#474).

## [0.28.0] - 2026-08-06

### Added

- An opt-in integration mirrors live Murmur captions to NotchPill while keeping
  the feature disabled by default and deleting the caption when switched off
  (#468).

### Changed

- Header, settings-navigation, native-spacing, and waveform fixes complete the
  history-workspace redesign introduced in 0.27.0 (#469).

## [0.27.1] - 2026-08-06

### Fixed

- Main-window chrome, transcript cards, and Settings now follow shared geometry
  tokens: native traffic lights align with the single header row, Record and
  status controls remain stable across recording states, and Copy no longer
  reserves space in transcript metadata (#466).

## [0.27.0] - 2026-08-06

### Changed

- The main window now uses one line of recording chrome and gives the transcript
  history the workspace. File transcription moved to drag-and-drop, the command
  palette, and the history overflow menu; usage stats moved to a compact footer
  with an Insights popover. Settings now uses four searchable tabs with a
  consolidated six-tab editor window, onboarding includes explicit hotkey setup,
  diagnostics uses the Events / Runs / Performance / Compare / Transform order,
  and the notch overlay supports persisted vertical calibration.
- Spoken structure and number formatting now share one deterministic transform,
  and obsolete audio states have been removed (#462, #463).

### Added

- Fleet log install pages now show the last proven microphone activation and
  last non-empty live transcription with relative and exact Eastern timestamps,
  derived from the complete retained event stream with bounded memory (#459).
- Fleet log install pages now offer exact latest-200/latest-500 JSONL downloads
  and LLM-ready Markdown reports with plain-English event meanings, grouped
  findings, bounded device context, and untrusted-telemetry guidance (#457).

## [0.26.0] - 2026-08-04

### Added

- The fleet diagnostics dashboard now leads with privacy-safe plain-English
  health for microphone capture, shortcuts, dictation, updates, and transforms;
  repeated warnings collapse into counted incidents, recovered fallback is
  distinguished from failure, and raw technical evidence remains expandable
  (#454).
- The vocabulary editor is more compact and easier to scan and edit (#453).

### Changed

- Dictation no longer pays the previous fixed latency floor after capture
  completes (#456).

## [0.25.4] - 2026-08-04

### Added

- Maintainers can request bounded, on-demand diagnostic probes from installs
  that explicitly opted into remote hang diagnostics (#452).

## [0.25.3] - 2026-08-04

### Added

- Consented installs can be armed for privacy-bounded microphone hang
  diagnostics, with safe truncation and explicit server control (#451).

## [0.25.2] - 2026-08-04

### Fixed

- Capture remembers first-attempt backend hangs, gives the primary attempt a
  bounded budget, and safely resets the slow-rescue preference (#450).

## [0.25.1] - 2026-08-04

### Fixed

- A session-scoped backend preference skips repeated AUHAL timeouts and records
  per-call capture timing without leaking device details (#445).

## [0.25.0] - 2026-08-03

### Changed

- Settings now uses the full workspace below the status bar: dashboard usage
  cards hide while Settings is open, and Performance begins directly with its
  diagnostics tabs instead of repeating a page title (#441).
- Settings now separates the local model **Benchmark** from an embedded
  **Performance** diagnostics workspace. Events, resource charts, run history,
  transform diagnostics, and report comparison live in the main window;
  diagnostics shortcuts no longer open a separate Log Viewer window (#435).

### Fixed

- Microphone initialization now gives AUHAL and CPAL separate bounded attempts,
  requires confirmed helper termination before fallback, honors Stop between
  attempts, suspends active deadlines during a pending macOS permission prompt,
  and reports privacy-safe setup sub-phases for root-cause attribution. This is
  a capture-contract repair, not a claim that the underlying Core Audio hang is
  eliminated (#436).

## [0.24.2] - 2026-08-02

### Fixed

- In-app updates now detect macOS Gatekeeper App Translocation before
  downloading, explain how to reinstall Murmur from Finder, and distinguish
  installation failures from update-check failures (#432).

## [0.24.1] - 2026-08-02

### Fixed

- The signed production capture worker now uses standalone microphone sandbox
  entitlements instead of invalid inheritance, preventing macOS from terminating
  it before protocol startup. Release builds execute the final packaged worker's
  hello/start handshake, and protocol startup failures no longer appear as
  unsupported microphone configurations or retry another backend (#428).

## [0.24.0] - 2026-08-01

### Added

- Production microphone capture now runs behind a signed, killable helper with
  capture-scoped binary PCM framing, an allocation-free SPSC callback boundary,
  CPAL and direct AUHAL backends, exact-device pre-buffer failover, deterministic
  fault modes, and partial-transcript recovery for interrupted recordings
  (#405, #408, #409, #410, #411, #412).
- A probe-only signed capture helper now has strict runtime code-identity
  checks, content-free callback health, deterministic cancel/kill/reap tests,
  and exact release provenance. Production recording remains unchanged pending
  the signed/notarized TCC matrix (#407).
- Update availability now stays visible without a backend or disruptive optional-update modal: Murmur performs due-gated checks on macOS wake and foreground activation, adds a native menu-bar check/update action, and keeps a versioned pill beside Record and Transcribe File until the release is installed or skipped.
- Dictated prose can now include bounded inline slash-command references such as `slash command chat` → `/chat` without reformatting the surrounding sentence.

### Fixed

- macOS microphone startup now tries the direct AUHAL capture path before CPAL,
  avoiding an unbounded CPAL stream-open stall observed with healthy USB default
  inputs. Content-free phase timings distinguish helper launch, stream open,
  first callback, first retained PCM, and stop-to-exit latency (#426).
- Clipboard auto-paste bypasses the slow accessibility no-value fallback when
  an application does not expose a writable accessibility target (#394).

## [0.23.9] - 2026-07-30

### Fixed

- Fresh installs no longer report their device state snapshot under a throwaway identity (#400).

## [0.23.8] - 2026-07-30

### Added

- Diagnostics now include an event-driven device state snapshot (selected microphone and connected audio inputs), sent only when it changes (#399).

## [0.23.7] - 2026-07-30

### Fixed

- Device names with typographic apostrophes no longer arrive mangled in diagnostic uploads (#398).

## [0.23.6] - 2026-07-30

### Fixed

- Development builds (`tauri dev`) no longer upload diagnostic logs (#397).
- Restored auto-updater release manifests; v0.23.3-v0.23.5 were published without them and could not be discovered by the updater.

## [0.23.5] - 2026-07-30

### Added

- Diagnostic batches now include machine specs (chip, RAM, core count), and the log shipper stays disabled in CI so release smoke tests no longer appear as phantom installs (#396).

## [0.23.4] - 2026-07-30

### Added

- Diagnostic log batches now include the Mac's device name, macOS version, and hardware model so the maintainer's fleet dashboard can label each install stream (#395).

## [0.23.3] - 2026-07-30

### Added

- Zero-config diagnostic log shipping: every install uploads its privacy-stripped structured event log (timing, errors, pipeline stats — never audio or transcription text) to the maintainer's receiver for debugging, identified only by a random install ID. Launch with `MURMUR_LOG_SHIPPER=off` to disable (#393).

## [0.23.1] - 2026-07-30

### Fixed

- A Core Audio stream build that never returns no longer leaves Murmur in `Recovering` until restart. Deadline-expired and user-cancelled generations close their callback gate, release logical microphone ownership immediately, and are joined by a background reaper if macOS eventually returns, so a fresh recording attempt can start without relaunching (#389).

## [0.23.0] - 2026-07-29

### Added

- History search now starts as a compact toolbar control, animates open on hover, remains pinned while focused or filtering, and supports predictable Escape/close behavior with accessible keyboard and reduced-motion states (#391).

### Fixed

- Developer vocabulary from project scans and built-in technical terms now activates only for apps explicitly configured with the Code / technical writing style or Local IDE context, preventing identifiers such as `toBe` and `all__` from leaking into ordinary prose (#390).
- Long What's New release notes now scroll independently while keeping the modal title and primary action visible (#385).

## [0.22.1] - 2026-07-29

### Added

- Microphone initialization now has explicit Starting and Recovering states, visible and cancelable startup, a 5-second still-connecting signal, a 30-second hard deadline, and truthful guidance when macOS audio teardown remains blocked (#380).
- Selected-text transform capture now remains in Connecting until both its frozen selection and exact microphone owner are ready; slow startup and recovery are visible, and releasing before readiness offers Retry instead of silently losing speech (#380).

### Fixed

- Slow Core Audio initialization can no longer detach timed-out capture threads or allow overlapping microphone owners. One supervisor retains and joins every worker, rejects starts during recovery, suppresses late readiness and samples from cancelled generations, and starts recording duration only after the stream is ready (#380).
- Display-change notification storms are coalesced for 125ms and ignored when the complete monitor/notch snapshot is unchanged, avoiding repeated overlay repositioning during an identical macOS event burst (#380).

## [0.22.0] - 2026-07-28

### Added

- After an automatic update relaunches, Murmur now shows a one-time What's New summary with the release's features and fixes. Generated GitHub release notes are embedded directly in the updater manifest so the same sanitized Markdown is available before and after installation (#379).
- **Local Appearance Theme Engine** adds System, Light, and Dark modes; accessible custom accent, background, foreground, and contrast controls; synchronized main/log-viewer semantic tokens and native title bars; flash-free cached first paint; exact Sonic reset; and bounded atomic JSON import/export that never touches the clipboard. Overlay and transform-review transparency and always-dark glass remain unchanged (#377).

### Fixed

- Escape now cancels selected-text transforms during capture, instruction listening, and local-model thinking, while ready/failed reviews retain their existing popover-local Escape behavior (#347).

## [0.21.1] - 2026-07-23

### Fixed
- Production app bundles now contain only the Murmur executable and signed local-LLM sidecar. The mock sidecar and local evaluation CLI remain available as Cargo examples for development and CI, and release finalization fails closed on any unexpected executable (#324).

## [0.21.0] - 2026-07-23

### Added
- **Selected-text Transform** captures an explicit selection, records a local instruction, runs a sandboxed on-device language-model sidecar, and presents a review-first proposal with Apply, Retry, Cancel, and bounded Undo. The flow includes independent hotkeys, presets and saved transforms, secure-field fail-closed behavior, clipboard restoration, concurrency guards, and privacy-safe pass tracing (#312, #332).
- **Persistent Performance and Runs diagnostics** replace the transient metrics view with bounded local run history, CPU and memory timelines, transform-stage correlation, incomplete-run rejection, and a dashboard for comparing latency and resource behavior without capturing dictated text (#351, #352).
- **Portable diagnostics reports** can be imported into a session-only Reports workspace for schema-validated inspection and side-by-side comparison. Imported data is never silently adopted into local run history, and invalid or oversized reports fail closed (#353).
- **Exact-term Correct and Teach** lets users select one heard term inside a longer sentence and review the precise replacement before saving it, avoiding accidental sentence-wide learned rules (#349).
- **Performance Lab report export** — each saved benchmark report can now be copied as full JSON, saved to a configurable folder (default `Documents/Murmur`) under a self-identifying `benchmark-<version>-<machine>-<createdAt>.json` name, and its folder revealed in Finder. An optional auto-save writes every completed run to disk so reports survive the 10-slot in-app cap. All local, no network (#308).
- **Local dictation evaluation harness** adds strict versioned fixtures, a deterministic no-hardware CI tier, an opt-in installed-model/audio tier, and machine-readable recognition/transformation/delivery reports through `murmur-eval` (#267).
- **Transform selection capture now works in Chromium/Electron apps** (Brave, Chrome, Slack, …): when the webview exposes no accessible selection — or its accessibility tree fails or times out entirely, as Chromium's routinely does — capture falls back to a sentinel-guarded synthetic Cmd+C that restores the clipboard afterwards. The fallback only reproduces the user's own copy gesture: positively detected password fields and denied Accessibility stay fail-closed, and secure fields refuse Copy system-wide, so against them the fallback can only time out, never read (#329).

### Fixed
- Transform diagnostics validate stage names and values before persistence, preserving the telemetry privacy boundary while still allowing performance correlation (#332).
- Imported diagnostics reject incomplete runs that claim success, preventing misleading comparisons in Reports (#353).
- Correct and Teach now uses uniquely provable case-insensitive context alignment, so harmless casing differences cannot turn a one-word correction into a broad sentence rule; ambiguous repeated-token alignments fail closed (#348).
- The transform instruction mic now arms the moment the key is pressed, before selection capture, instead of after it. In Chromium apps capture can take over a second (accessibility warm-up retries + the clipboard fallback), and arming afterwards chopped the start off the spoken instruction — the reproducible "Didn't catch an instruction" in browsers. Audio from an aborted capture (secure field, error, cancel) is stopped and never transcribed (#329).
- A transform keypress that is refused because dictation, a benchmark, a file transcription, or another transform owns the pipeline now flashes an amber busy indicator on the overlay instead of being silently ignored (#329).
- Pressing the dictation key with a **ready** (Approve/Retry) transform popover open now dismisses the review — discarding the unaccepted proposal, exactly as Cancel would — and starts recording, instead of silently refusing until the popover was dismissed manually. Extends the #327 failed-review auto-dismiss to all reviews (#329).
- Rapidly pressing the transform key can no longer surface the hidden main window: the popover focus guard now uses a sticky main-window-visibility snapshot taken at the first popover show of a pass, instead of a per-call snapshot that could observe (and then preserve) a transiently-surfaced main window (#329).
- A failed transform review popover (sidecar crash, blank instruction, capture error, missing model) no longer blocks dictation until manually dismissed: pressing the dictation key now auto-dismisses the failed review — which holds nothing user-approvable — and records (#327; extended to ready reviews by #329 above).
- The selected-text transform popover no longer crashes the app on macOS 26. Its `NSWindow` level/activation and shadow treatment were mutated directly from async command context (a tokio worker); macOS 26 hard-traps on off-main `NSWindow` mutation. Those raw AppKit calls are now dispatched to the main thread via `run_on_main_thread`, matching the app's AX write paths (#325).
- Disabling Murmur from the overlay no longer traps it disabled: the hover quick-settings card — which holds the "Enable Murmur" power button — now stays reachable while disabled, so the overlay's own control can turn Murmur back on. Previously the global-disable state gated off the overlay's hover-expand, leaving the tray "Disable Murmur" check item as the only way back.

## [0.19.0] - 2026-07-20

### Added
- **Unified Settings and trustworthy Performance Lab state** reorganizes Settings into six task-oriented pages, adds a bounded memory-only running-app picker with manual bundle-ID fallback, and records privacy-safe environment/corpus/execution metadata in versioned benchmark reports while retaining legacy saved runs (#258).
- **Correct and Teach** lets users edit the newest local history entry, review one bounded high-confidence replacement, and explicitly save it as global, app, or unambiguous project-scoped knowledge. Learned rules persist locally, run deterministically through Smart Correction, and remain inspectable, editable, disableable, exportable, and deletable in Knowledge; ambiguous edits and Voice Command conflicts fail closed (#251).
- **Teach specific term** adds an explicit Correct and Teach escape hatch for selecting or entering one exact heard term and written replacement when automatic extraction is too broad or safely fails closed. The local review shows whole-term occurrences, before/after output, and scope while preserving confirmation-only persistence and existing knowledge and Voice Command conflicts (#349).
- A single transcription model catalog and runtime manager now expose backend capabilities, platform/install state, serialized load/warm/readiness/unload lifecycle, and privacy-safe generation-ordered status events for all seven shipped models (#247).
- **Voice Commands 2.0** upgrades legacy phrase replacements into typed, persistent local commands with multiline snippets, deterministic date/time variables, explicitly permitted clipboard insertion, global/per-app scopes, conflict validation, and a no-paste preview/test UI. Existing pairs migrate idempotently and retain built-in-first literal behavior (#248).
- **Performance Lab trust overhaul** — benchmark scoring now reports three tiers per model and fixture: raw decoder WER, normalized WER (digit/word, unit, and compound formatting differences no longer count as errors, #270), and delivered WER measuring the text after the production transform pipeline with the whisper dev-vocab prompt applied (#271). Five new stress fixtures (jargon, numbers, disfluent, 64s extra-long, fast speech) de-saturate model ranking (#273), one-time shared Metal/ANE init is measured separately instead of being charged to the first model loaded (#274), and a headless runner (`tests/headless_benchmark.rs`) produces full benchmark reports from the command line. Validation data and remaining caveats: `docs/investigations/benchmark-validation-2026-07-20.md`.
- **Persistent personal knowledge store** keeps replacement rules, vocabulary terms, and snippets in a versioned local SQLite database with deterministic migration/backup recovery. Settings provides bounded search, scoped inspection, create/edit/enable-disable/delete, atomic export/import, visible recovery state, and confirmed delete-all; transcription, Correct and Teach, and command execution remain separate future integrations (#246).
- **Explicit spoken vocabulary aliases** let users map exact recognized variants such as `Tori` and `Tory` to a canonical written term such as `Tauri`. Structured entries migrate existing vocabulary, validate ambiguity/cycles/command conflicts, run locally across every backend before fuzzy and CLI formatting, and include an in-memory Settings preview (#268).
- Opt-in per-app **Local IDE symbols and `@file` context** builds a bounded memory-only index from user-selected roots, corrects unique project symbols, and canonicalizes explicitly triggered file mentions to root-relative text. It never reads screen, selection, or clipboard context; ambiguous or stale references stay unchanged, and reviewed CLI formatting remains authoritative (#253).
- Per-app **Writing Styles** add explicit Inherit, Conversational, Polished prose, Code / technical, Verbatim, and Notes policies using only local deterministic transforms. Styles resolve once in the immutable recording context, never infer app type or capture app content, and preserve existing delivery behavior (#250).
- **Spoken CLI command formatting** — likely npm/npx, Git, Cargo, Docker, kubectl, and other developer commands now receive deterministic local formatting for versions, flags, paths, operators, quotes, and small canonical aliases. Detection is prefix/trigger/profile bounded, project `package.json` names extend the local lexicon, and ordinary prose remains unchanged (#256).
- Optional **Smart Formatting** turns clear spoken enumerations into lists, applies explicitly cued email/URL/symbol/quote/paragraph grammar, and handles bounded same-utterance restatements locally. It is independently controllable per app, bypasses CLI/code/verbatim contexts, leaves imported-file transcription raw, and keeps delivery final-only (#252).

### Changed

- Settings now uses one model selector, presents clipboard-first delivery and file-output suppression explicitly without overwriting the stored auto-paste preference, and uses semantic surface tokens throughout Performance Lab. The strict Fastest and Balanced recommendation contract is unchanged (#258).
- Onboarding, Settings, recording preparation, model downloads, and Performance Lab now consume the shared catalog/runtime contract. Unknown model identifiers fail closed, and model failures never trigger automatic cross-model fallback (#247).
- All supported backends now use one final-after-stop transcription path. The Whisper-only incremental worker, provisional overlay preview, preview setting, lifecycle events, reconciliation code, and incremental telemetry were removed; final clipboard, paste, file output, history, and stats delivery remains exactly once (#279).
- Post-recognition cleanup, voice commands, and Smart Correction now run through one ordered, backend-neutral transformation pipeline with privacy-safe per-stage timing/change telemetry and explicit failure policy (#244).

### Fixed
- Long Whisper batch and file transcriptions now retain timestamp-based continuation, preventing an early end-of-text token from silently dropping the remaining audio while preserving single-segment decoding for short audio (#269). Multi-segment output is additionally guarded against words gluing together at segment joins.
- Performance Lab recommendations now rank Fastest by the strict minimum duration-weighted realtime factor, while Balanced uses normalized WER within two accuracy points, an inclusive 10% speed band, then lower memory with deterministic ties (#272).
- Per-app profile matching now uses the native macOS frontmost-application query with bounded retries and a timeout-bounded compatibility fallback, while preserving one immutable recording-start snapshot and privacy-safe detection telemetry (#265).

## [0.17.2] - 2026-07-19

### Added
- Optional **Hotkey Timing Feedback** flashes the overlay amber when a bare-modifier tap times out before its second tap in Double-Tap or Both mode. The setting is off by default, and intentional holds, modifier shortcuts, processing skips, and valid double-taps remain silent (#154).

### Changed
- The notch overlay is minimal again: idle sits flush with the notch showing only the small mic tab on the left, recording expands to the right with the red dot and live waveform, and processing shows just the spinner instead of a row of static dots. The hover quick-settings card still exists but is now intent-gated — it opens only after the cursor dwells on the island for 150 ms (no more popping open on a graze) and is more compact. A transparent-background regression from the Sonic Canvas reskin that painted the whole overlay window as a dark box is fixed, and global disable is now also available as a "Disable Murmur" check item in the tray menu.
- The main window, settings, transcription history, recording controls, and log viewer now use the Sonic Canvas surface hierarchy and semantic palette in light and dark appearances (#141).
- Release automation now builds signed macOS and Linux artifacts once on trusted `main`, keeps Cargo/CUDA cache ownership off tags and pull requests, and promotes only commit-SHA-matched artifacts with fail-closed updater-signature checks (#220).
- Successful trusted version-bump builds now automatically create the matching tag and publish their already-verified artifacts; manual builds remain rehearsals and tag pushes remain a recovery path (#239).

### Fixed
- The setup assistant's model step now detects every already-downloaded model, badges installed rows, and offers Continue instead of Download for them; the wizard is also skipped entirely when permissions are granted and any model exists on disk (#240).
- Consecutive Core ML dictations now start with fresh Parakeet decoder state, preventing later one-shot recordings from collapsing to punctuation-only empty transcripts (#236).
- `murmur-diag` now reads and source-labels both release and dev log streams without duplicate file ingestion, keeps cross-build correlation isolated, and uses one documented user-level MCP registration instead of per-worktree registrations (#191).
- Code-vocabulary scans now keep the View-all dialog keyboard focus contained and restore the opener on close, correlate live progress by scan ID, and report superseded results when settings change during a walk instead of presenting non-adopted terms as complete (#209).
- Global modifier hotkeys now recover when macOS disables the underlying event tap, avoid stale modifier-state dead zones after system context changes, and no longer process mouse movement or perform main-thread key-name translation on the modifier hot path (#194, fixes #137).
- Quick Both-mode holds now stop and transcribe as soon as the 200 ms promotion threshold is reached instead of being discarded by an obsolete 300 ms grace window; empty Core ML results after VAD also retry once with the original audio and emit privacy-safe diagnostics (#221).
- Fast hold-down dictations no longer disappear when key release races Core Audio startup; native start, stop, and cancel transitions are serialized and the frontend waits for startup before processing (#216).
- Parakeet v2 downloads now survive an interrupted extraction: Murmur reuses the completed archive, validates a staged bundle, and publishes it atomically instead of leaving a partial model that appears undownloaded (#215).
- Core ML model setup now shows an animated indeterminate Installing state across onboarding, Settings, and Performance Lab instead of a frozen 0% bar (#217).

## [0.16.0] - 2026-07-17

### Added
- **In-app transcription Performance Lab** — benchmark installed models against bundled audio fixtures from Settings, with scoring, busy-state isolation, and lifecycle management (#212, #213).
- **First-launch setup assistant** — new installs get a guided wizard (Welcome → Microphone → Accessibility → Model download → Done) instead of a dismissible permissions banner next to a lone model-download screen. The microphone step fires the native macOS permission dialog in-app (new `request_microphone_access` command via `AVCaptureDevice.requestAccess`) instead of waiting for the first recording attempt; both permission steps poll live so a grant made in System Settings flips the step when you come back, and denied/stale-TCC states get inline reset-and-retry paths. Existing installs with permissions and a model already in place are grandfathered silently. Re-run anytime via Settings → About → Run Setup Assistant (`OnboardingFlow.tsx`, `lib/onboarding.ts`).

## [0.14.1] - 2026-07-16

### Changed
- Migrated installed clients to the `latest-v2.json` updater channel while retaining macOS 13 compatibility. This bridge release keeps automatic updates working before Murmur's macOS 14 transition.

## [0.14.0] - 2026-06-24

### Added
- **Live code-vocabulary scan feedback** — choosing a project folder now shows a live scan strip: a breadth-first walk streams files and skipped directories as it indexes, with running counts, a cap warning when the walk truncates, and the top terms found. Replaces the previous silent, feedback-free scan (`VocabScanStrip`, `useVocabScan`, `scan_code_vocab`).
- **View-all scanned terms pop-out** — a searchable, sortable modal listing every kept identifier with its frequency, split into the top-96 that feed Whisper's prompt and the remainder that feed Smart Correction (`VocabTermsModal`).
- **Decoupled vocabulary budgets** — Whisper's initial prompt stays token-bound at the top 96 terms, while Smart Correction now consumes the top 500 (no token limit) — a large recall win for post-recognition correction on every engine.

### Changed
- **Breadth-first folder scan** — the walk now samples across sibling projects (FIFO, name-sorted) instead of depth-diving the first subdirectory, so a parent folder like `~/code` indexes fairly. Walk caps raised to 1000 files / 32 MB (per-file 512 KB unchanged).
- **Bounded scan memory** — identifiers are extracted per file during the walk and the contents dropped, so memory is bounded by the unique-term count rather than total bytes scanned.
- Whisper's initial prompt is now deduplicated across folder-scan, built-in, and custom sources so a term never burns two slots of the token budget.

### Fixed
- **Smart Correction no longer re-fragments its own output** — Tier-2 fuzzy tokenization treats `_` as a word character, so a snake_case form produced by Tier 1 (e.g. `error_message`) is no longer split and a sub-token fuzzy-rewritten (`error` → `Errorf`).
- **Tier-2 fuzzy over-correction** — only structured identifiers (camelCase / snake_case / digit) are fuzzy-eligible; plain words (`Errorf`, `Record`, `kubectl`) are exact-match only, so dictating ordinary English no longer flips to a scanned identifier.
- Smart Correction rebuilds with folder terms on the lazy path after restart (previously stayed built-in-only until an unrelated settings change).

## [0.13.0] - 2026-06-23

### Added
- **Smart Correction** — vocabulary is now applied to the transcript *after* recognition, on **every** engine (including the default Parakeet, which ignores Whisper's prompt). Tier 1 is an exact spoken→written map (Aho-Corasick, single pass) that turns "use effect" into `useEffect`; Tier 2 is opt-out "sounds-like" matching (phonetic key + edit distance, fires only near your vocabulary) that recovers close mishearings like "red pivot" → `rePivot`. Built once on settings-change, runs inline in well under a millisecond (logged as a `correction_ms` telemetry phase). Common dev abbreviations (e.g. "standard error" → `stderr`) are included when Code-Aware Vocabulary is on. Settings: Vocabulary → Smart Correction (on by default) + Sounds-like matching sub-toggle (`correction.rs`).

### Changed
- Code-Aware Vocabulary now also corrects the transcript on every backend via Smart Correction, not just Whisper's prompt.

## [0.12.0] - 2026-06-23

### Added
- **Overlay hover-expand quick settings** — hovering the Dynamic Island reveals a quick-settings dropdown with global-disable, auto-paste, and settings-window controls; inline recording timer while hovering (#135)
- **Accessibility permission reset** troubleshooting button in the permissions banner — resets the app's stale TCC entry for the current bundle id (`tccutil reset Accessibility`) and reopens System Settings
- **Save dictation output to file**: optional "Save Transcript to File" (`.txt`) and "Save Audio to File" (`.wav`) toggles for live hotkey dictation, with a configurable output folder (defaults to `Documents/Murmur`). When either is enabled, text is still copied to the clipboard but auto-paste is paused (`file_output.rs`).
- **History source badge**: each history entry now shows whether it came from live recording ("Mic") or a transcribed file ("File", with the source file name).
- **Built-in code vocabulary** — code-aware vocabulary now works with no folder selected, biasing transcription toward a curated dev-term dictionary (`useEffect`, `kubectl`, `stderr`, …); an optional project folder layers your own identifiers on top (`vocab::builtin_terms_prompt`).
- **Custom voice commands** — define your own spoken `phrase → replacement` pairs (applied after the built-in commands) in Settings (`voice_commands::apply_voice_commands_with_custom`).
- **Transcript cleanup sub-toggles** — independently turn off "remove filler words" and "capitalize sentences" while keeping cleanup on.
- **Per-app transcript-cleanup override** — per-app profiles can now force cleanup on/off per frontmost app, alongside the existing auto-paste override.

### Changed
- **Unified Vocabulary settings** — the manual Custom Vocabulary input and the Code-Aware Vocabulary controls now live together in one "Vocabulary" section (both feed the same Whisper initial prompt).

### Fixed
- **Microphone permission banner no longer false-negatives** after a dev rebuild or app move (stale TCC, #190). The banner now reads the live 4-state `AVCaptureDevice` authorization status and treats `notDetermined`/`unknown` as transient (not a hard "denied"), so a stale TCC entry can't mislabel a working mic. Added a microphone **reset** troubleshooting button (`tccutil reset Microphone <bundle-id>`) mirroring the Accessibility reset.
- Strip recording-status-changed emissions from `ensure_vad_model` to reduce event noise

## [0.11.0] - 2026-06-19

### Added
- **Insights dashboard** — usage analytics view surfacing words, WPM, recordings, and token metrics (#196)
- **Per-app profiles** — frontmost-app detection drives per-application dictation settings and behavior (#199)
- **Voice commands** — spoken command recognition during dictation (#197)
- **AI cleanup** — post-transcription text cleanup pass (#198)
- **Multi-language support** — configurable default language and additional language selection (#200)
- **Multi-file drag-and-drop** — queue and transcribe multiple audio files via drag-and-drop (#201)
- **Code-aware vocabulary** — vocabulary biasing for code and technical terms (#202)

### Fixed
- **Microphone permission stale-TCC fix** — banner no longer false-negatives from a stale TCC entry (#204)
- **Auto-paste `.textClipping` fix** — corrects clipboard/auto-paste handling to prevent `.textClipping` artifacts (#203)

## [0.8.0] - 2026-03-02

### Added
- **Structured event system** with `TauriEmitterLayer`, ring buffer, JSONL export, and privacy stripping (`telemetry.rs`)
- **Log viewer window** with Events and Metrics tabs for real-time structured event inspection

## [0.7.8] - 2026-03-01

### Fixed
- Cache `WhisperState` to eliminate per-transcription alloc/free cycles, improving latency

## [0.7.7] - 2026-03-01

### Added
- **Collapsible accordion sections** for the settings panel
- **Pre-VAD RMS logging** and VAD sensitivity slider for tuning speech detection

## [0.7.6] - 2026-02-28

### Fixed
- CI: set `CMAKE_OSX_DEPLOYMENT_TARGET=11.0` to fix `std::filesystem` errors with Xcode 16.4
- CI: add ARM i8mm flags to rust check job

## [0.7.5] - 2026-02-28

### Added
- **Silero VAD pre-processing** to filter silence and prevent whisper hallucination loops (`vad.rs`)
- **Configurable auto-paste delay** with retry logic and failure notification

### Fixed
- Discard phantom recordings and add transcription logging
- Reposition overlay on display configuration change

### Changed
- Split `lib.rs` into focused single-responsibility modules (`state.rs`, `audio.rs`, etc.)
- Split `keyboard.rs` into focused submodules
- Rename `ui/` to `app/` at repo root

## [0.7.0] - 2026-02-27

### Added
- **"Both" recording mode** — simultaneous hold-down + double-tap (`useCombinedToggle.ts`)

### Fixed
- Allow scrolling within long transcription history entries
- Restore tray icon and fix overlay click surfacing main window

## [0.6.7] - 2026-02-27

### Changed
- **Rebrand to Murmur** — app rename with new icon

## [0.6.5] - 2026-02-26

### Added
- **OTA auto-updater** with min-version enforcement (`useAutoUpdater.ts`, `lib/updater.ts`)
- Custom styled select dropdowns replacing native selects

### Fixed
- Log accessibility permission status in keyboard listener start/stop

## [0.6.2] - 2026-02-26

### Added
- **Microphone device selection** in settings
- **Launch at login** toggle

## [0.6.0] - 2026-02-26

### Added
- **Interactive overlay** with Dynamic Island notch integration (`OverlayWidget.tsx`, `commands/overlay.rs`)

## [0.5.3] - 2026-02-24

### Added
- Group model selector by backend (Moonshine / Whisper)
- CI: Rust tests and settings migration tests in CI pipeline
- CI: post-build smoke test in release workflow

### Fixed
- Statically link sherpa-rs to fix launch crash

## [0.5.0] - 2026-02-23

### Added
- **Moonshine transcription backend** via sherpa-rs as an alternative to Whisper
- **Hold-Down recording mode** replacing Key Combo mode (press to start, release to stop)
- `TranscriptionBackend` trait extracted from `transcriber.rs` for backend abstraction

### Fixed
- Eliminate auto-paste toggle race conditions and silent failures
- Surface Control shortcut failures and warn in settings

## [0.4.0] - 2026-02-20

### Added
- **In-app model downloader** for first-launch onboarding
- Per-phase timing instrumentation for the transcription pipeline

### Fixed
- Surface rdev listener failures and add heartbeat logging

## [0.3.2] - 2026-02-19

### Fixed
- Auto-paste toggle shrinks and loses track in dark mode
- Set `signingIdentity` so local builds use Developer ID cert
- Use draft-then-publish pattern in release workflow

## [0.3.0] - 2026-02-19

### Added
- **Live resource monitor** with CPU/memory chart (`resource_monitor.rs`, `useResourceMonitor.ts`)
- **Logging viewer** for inspecting app logs in real time
- **Double-tap modifier key recording mode** — double-tap Shift/Option/Control to start recording, single tap to stop
- **Recording mode setting** — choose between "Key Combo" and "Double-Tap" modes in Settings
- Unit tests (23 tests) for the `DoubleTapDetector` state machine
- `keyboard.rs` module for double-tap detection and rdev listener management

### Fixed
- Settings help text incorrectly described recording behavior
- rdev macOS crash: switched to git `main` branch and added `set_is_main_thread(false)` to prevent TIS/TSM segfaults

### Changed
- Accessibility permission now also required for double-tap recording mode

## [0.2.0] - 2026-02-19

### Added
- Native audio capture via cpal (replaced Web Audio + Python sidecar)
- Pure Rust transcription pipeline via whisper-rs with Metal GPU acceleration
- Auto-paste toggle with osascript Cmd+V simulation
- File-based logging with rotation (`~/Library/Application Support/local-dictation/logs/`)
- Word statistics with stats bar and localStorage persistence
- Custom hotkey binding
- Status widget — tray icon, overlay pill, audio waveform
- Warm neutral UI redesign

### Removed
- Python sidecar dependency — all processing is now pure Rust
- Web Audio capture module (`audioCapture.ts`)

## [0.1.0] - 2026-02-19

### Added
- Tauri desktop app with React/TypeScript frontend
- System tray integration (menubar icon)
- Global hotkey support (Shift+Space, Option+Space, Control+Space)
- Settings panel (model selection, hotkey configuration)
- Transcription history with copy-to-clipboard
- Recording status indicator with duration timer
- macOS permissions guidance
- About window with version info
- Production build with DMG installer
- Python sidecar for transcription (whisper.cpp)
- JSON-based communication protocol
- Support for multiple Whisper models (tiny.en to large-v3-turbo)
- Local processing with no cloud dependencies
