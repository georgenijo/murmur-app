# Competitive dictation audit — August 2026

## Purpose

This audit compares Murmur with Wispr Flow, Superwhisper, FluidVoice,
VoiceInk, Monologue, Aqua Voice, Willow, and MacWhisper. It converts the
comparison into one implementation wave split between Linux-first product
foundations and Mac-native integration work.

Competitor claims are product-positioning evidence, not independent accuracy
or latency measurements. Reproduce material claims with Murmur Bench and
native acceptance tests before using them in marketing.

## Executive finding

Murmur is already strong in offline privacy, deterministic formatting,
developer vocabulary, safe rewriting, capture ownership, diagnostics, and
benchmarking. Market leaders feel ahead mainly because their capability is
easier to see and operate:

- live words appear while the user is speaking;
- one named mode collects model, style, context, and delivery behavior;
- history exposes original and polished results and can be processed again;
- recording and delivery provide immediate audio and recovery feedback;
- meeting transcripts produce summaries, decisions, and action items.

The next wave should improve those workflows without implicit screen capture,
cloud-required inference, or ambiguous auto-application.

## Market comparison

Legend: **Yes** = public support, **Partial** = narrower or materially different,
**No** = no public support found.

| Capability | Murmur | Wispr Flow | Superwhisper | FluidVoice |
|---|---|---|---|---|
| Any-app dictation | Yes | Yes | Yes | Yes |
| Fully offline dictation | Yes | No | Yes | Yes |
| Deterministic local formatting | Yes | No | Partial | Partial |
| Live transcript preview | No | Partial | Cloud models | Yes |
| Named user modes | Partial | Partial | Yes | Yes |
| Automatic behavior by app | Yes, profiles | Yes | Yes | Yes |
| Automatic behavior by website | No | Yes | Yes | Partial |
| Personal vocabulary | Yes | Yes | Yes | Partial |
| Bounded learned corrections | Yes | Partial | Partial | No |
| Code and terminal formatting | Yes | Yes | Partial | Partial |
| Selected-text rewriting | Yes | Yes | Yes | Yes |
| Review before rewrite applies | Yes by default | Optional | Partial | Partial |
| Preserve raw and delivered text | No | Partial | Yes | Yes |
| Reformat or retranscribe history | No | Partial | Yes | Partial |
| Delivery retry / paste last | No | Yes | Yes | Yes |
| Recording sound cues | No | Yes | Yes | Yes |
| Meeting capture | Yes | Rolling out | Yes | No |
| Private mic/system channel split | Yes | Partial | Yes | No |
| Speaker diarization | Planned | Cloud | Yes | Partial |
| Meeting summaries/actions | No | Rolling out | Yes | Partial |
| Local performance/WER lab | Yes | No | No | No |
| Cross-device applications | No | Yes | Yes | Planned |

## Product principles

1. Keep final delivery authoritative and exactly once. Live text is provisional
   overlay content only.
2. Keep clipboard-first recovery. Clipboard restoration is not in this wave.
3. Preserve raw recognition and delivered text locally, but do not retain normal
   dictation audio unless the user explicitly opts in.
4. Present profile and pipeline capability as one comprehensible Mode; do not
   duplicate underlying settings or knowledge.
5. Resolve app activation once at recording start. Site-aware activation stays
   out of scope until its privacy boundary is designed.
6. Meeting summaries are derived artifacts. The transcript remains authoritative,
   and extracted actions link back to source segments.
7. Do not block product work on Usher or broader multi-agent infrastructure.

## Epic A — Linux product foundations

### L1. Preserve raw recognition and delivered text

- Add a versioned history shape with raw ASR text, delivered text, model,
  resolved mode/profile identity, stage outcomes, and recording correlation.
- Migrate existing history without inventing raw text; keep audio absent by default.
- Add privacy, retention, and export tests.

### L2. Introduce the Murmur Mode domain model

- Define one reusable mode referencing writing style, pipeline stages,
  vocabulary/context policy, model/language policy, and delivery behavior.
- Ship Everyday, Messages, Email, Notes, Technical, Terminal, and Verbatim.
- Preserve immutable recording-start resolution and migrate current profiles
  without changing delivered behavior.

### L3. Build the Modes user experience

- Create, duplicate, rename, edit, enable, and delete custom modes.
- Show effective behavior, bind a mode to multiple apps, and provide
  before/after testing without injection.

### L4. Reformat historical text with another Mode

- Run preserved raw text through an explicitly selected compatible Mode.
- Preserve the original and create a derived result with provenance.
- Do not describe this as retranscription or increment statistics/learning.

### L5. Define private meeting summaries and exports

- Add bounded chunking and hierarchical merge logic for long meetings.
- Define summary, decisions, action items, open questions, and supporting
  segment IDs with a strict schema.
- Add Markdown, plain-text, and JSON exports; unknown owners/dates stay unknown.

## Epic B — Mac-native experience

### M1. Add recording and delivery sound cues

- Configurable start, stop, success, and failure cues with preview and volume.
- Play start only after capture ownership is accepted.
- Never delay capture/delivery; suppress meeting cues by default.
- Verify cues do not contaminate retained or transcribed microphone audio.

### M2. Add Paste Last / Retry Delivery

- Reinsert the latest delivered text without recording or retranscription.
- Expose it in the command palette and tray with an optional shortcut.
- Reuse secure delivery checks; do not change history, statistics, or learning.

### M3. Add native Mode switching and app activation

- Show the resolved Mode in the overlay and tray.
- Allow manual cycling and temporary override.
- Activate app bindings using the existing bundle-ID snapshot and restore the
  last manual Mode after leaving an automatically bound app.
- Do not add site or screen-content capture in this wave.

### M4. Add safe live Parakeet preview

- Adapt Voice Query's generation-gated partial-decode pattern for dictation.
- Start with the supported Core ML/Parakeet route.
- Send provisional text only to the overlay; never paste, copy, persist, export,
  log, or count it.
- Bound cadence, trailing audio, CPU/RSS, and one-in-flight ownership; prove
  final delivery remains exactly once.

### M5. Execute private meeting summaries locally

- Run the Linux-defined pipeline through the signed local sidecar.
- Add start, progress, cancellation, retry, and derived-result storage.
- Serialize model ownership, link results to source segments, and measure
  long-meeting runtime and peak RSS on the Mac mini.

## Dependency order

- L1 precedes L4.
- L2 precedes L3, M3, and L4; L3 also precedes M3.
- L5 precedes M5.
- M1, M2, and M4 can begin immediately.

The Linux epic can execute L1 and L2 in parallel, then L3/L4, while L5 runs
independently. The Mac epic can execute M1, M2, and M4 immediately; M3 waits for
L2/L3, and M5 waits for L5.

## Evidence gates

Every child issue must provide focused tests, privacy-boundary tests for newly
retained or displayed content, self-review against acceptance criteria, and
exact commit-SHA evidence in its pull request.

Mac-native issues additionally require a native app smoke test, screenshots or
a short recording, content-free logs and timings, and Murmur Bench when
recognition latency, accuracy, delivered output, or memory can change.

## Sources

- [Wispr Flow features](https://wisprflow.ai/features)
- [Wispr Flow context awareness](https://docs.wisprflow.ai/articles/4678293671-feature-context-awareness)
- [Superwhisper modes](https://superwhisper.com/docs/modes/modes)
- [Superwhisper history reprocessing](https://superwhisper.com/docs/get-started/transcribe-history)
- [Superwhisper advanced delivery](https://superwhisper.com/docs/get-started/settings-advanced)
- [FluidVoice](https://github.com/altic-dev/FluidVoice)
- [VoiceInk](https://github.com/beingpax/VoiceInk)
- [Monologue](https://www.monologue.to/)
- [Aqua Voice](https://aquavoice.com/)
- [Willow](https://willowvoice.com/)
- [MacWhisper](https://www.macwhisper.com/)

Murmur's authoritative shipped baseline remains
[docs/FEATURES.md](../FEATURES.md).
