# Local Dictation Evaluation Harness

`murmur-eval` runs repeatable, local evaluations across three separate boundaries:

1. raw ASR text against a curated expected transcript or bounded alternatives;
2. final text from the production transformation pipeline, including per-stage outcomes;
3. final text written to an in-memory delivery sink.

It does not read Murmur history, app settings, the microphone, the system clipboard, or frontmost-window state. It does not upload fixtures or reports.

## Commands and tiers

Run the deterministic suite from `app/src-tauri`:

```bash
cargo run --bin murmur-eval -- deterministic \
  --output target/murmur-eval/deterministic-report.json
```

This is the CI tier. It uses fixture-provided raw ASR, a fixed clock, a fake clipboard-backed `VoiceCommandRuntime`, fixture-only profile/IDE context, and an in-memory delivery sink. It has no microphone, model, Metal, network, clipboard, paste, or window dependency.

The hardware tier is explicit and opt-in:

```bash
cargo run --bin murmur-eval -- hardware \
  --machine-label local-mac \
  --output target/murmur-eval/hardware-report.json
```

Hardware fixtures reference repository-owned WAV files and an exact backend/model. The runner refuses missing models instead of downloading them, restricts audio to the selected workspace root, and records OS, architecture, logical CPU count, user-supplied machine label, model/backend/accelerator, latency, and memory metadata where available. Missing installed hardware is reported as skipped.

## Fixture contract

Fixtures live under `app/src-tauri/eval/fixtures/{deterministic,hardware}`. Every `.json` file may contain one strict fixture object or an array of fixtures. `fixtureVersion` is currently `1`; unknown fields, unsupported versions, duplicate IDs, tier mismatches, and incomplete provenance fail before evaluation.

Each fixture declares:

- an ID and tier;
- local provenance, deletion guidance, and `containsRealUserData: false`;
- fixture-provided raw ASR for deterministic runs, or a workspace-relative WAV plus exact installed model/backend for hardware runs;
- a recognition reference transcript and one or more bounded acceptable raw-ASR outputs (the first bounded output is the reference when `referenceTranscript` is omitted);
- fixture-only stage switches, profile metadata, vocabulary, voice commands, CLI prompt, and IDE symbols/files;
- expected final and delivered text plus selected stage outcomes;
- fixed deterministic timing values.

Adding a case requires only another valid JSON object; evaluator code does not contain a fixture registry.

The repository corpus covers standalone and longer-sentence backtracking, numbered and bulleted lists, spoken punctuation/paragraphs, npm/npx/Git/Cargo/Docker/kubectl command formatting, Tauri/Tori/Tory aliases, Cursor profile/project file and symbol context, fixed date/time plus fake clipboard snippets, long final-only behavior, and ordinary-prose false-positive cases.

## Metrics and final-only behavior

Reports are versioned JSON and keep recognition, transformation, and delivery results separate. They include raw and normalized WER (using the same normalization as Performance Lab), CER, bounded-alternative match, exact command/final/delivery match, no-change preservation, stage outcome/change/text observations, fixed or measured latency, fallback stages, and memory metadata when available.

Stage text observation exists only inside the evaluator and remains in the local report; production telemetry continues to log privacy-safe stage metadata without transcript text.

Murmur no longer has incremental transcription or live preview. Every report therefore uses:

```json
{
  "partialCount": 0,
  "firstPartialMs": null,
  "firstPartialApplicability": "notApplicable",
  "finalOnly": true,
  "incrementalCompletion": "notApplicableFinalOnly"
}
```

Fixtures that claim partial output are rejected.

## Privacy, provenance, and deletion

Only explicitly curated synthetic or repository project-test fixtures are accepted. `containsRealUserData: true` is rejected, there is no importer for transcription history, and hardware audio must resolve inside the chosen workspace. The runner never uploads data and never reads the real clipboard; fixture clipboard text is an in-memory fake used only after a permitted fixture command matches.

Reports may contain the curated fixture transcripts and per-stage text, so keep them local. By default they are written under ignored `app/src-tauri/target/murmur-eval/`. To delete evaluation data, remove the generated report. To remove a fixture, delete its JSON entry and any explicitly referenced repository WAV. No database or hidden evaluator cache needs cleanup.
