# #540 diarization prerequisite spike

Date: 2026-09-07
Repository revision inspected: `9d169af6896269031612ee5ead120fa3df07f0ef`
Conclusion: FluidAudio's pinned offline diarizer is usable and fast enough on the target M4 Mac. The feature can proceed after its model, audio-lifetime, attribution, and scheduling contracts are implemented explicitly. No existing fixture or performance result in Murmur satisfied the issue before this spike.

## Acceptance evidence

| Requirement | Evidence now | Status for implementation |
|---|---|---|
| FluidAudio availability at Murmur's pin | `fluidaudio-rs` exposes synchronous `init_diarization` and `diarize_file`; the bridge uses `OfflineDiarizerManager` | Feasible |
| Exact model size | Required offline set is 21,599,417 bytes at model revision `1ed7a662...` | Measured |
| Processing time per audio hour | 15.40 s cold inference, 11.57 s warm inference on M4, excluding 8.97 s initialization | Measured |
| Peak RSS | 457,687,040 bytes, 436.48 MiB, for initialization plus two full passes; zero swaps | Measured |
| Trustworthy multi-speaker fixture | AMI `ES2004a.Mix-Headset.wav`, official RTTM and UEM, four labeled speakers | Available and staged |
| Stable within-session labels | Two runs produced the same four canonical labels and the same 94 boundaries at millisecond precision | Measured |
| Speaker accuracy | A small independent 10 ms scorer measured approximately 7.15% DER with a 0.25 s collar and reference overlap ignored | Promising, approximate only |
| Mic stays `Me` | The API was run only on the remote fixture. The integration must keep the mic path outside diarization. | Design prerequisite |
| Rename and session isolation | Current storage has only per-session `Me` and `Them` labels | Not implemented |
| Failure and absent-model fallback | Current meeting rows support plain `Them`, but no diarization lifecycle exists | Not implemented |
| Install, status, and remove | The transcription catalog supports install and status. It has no general `remove_model` command. | New auxiliary-model path required |
| No persisted embeddings | The Rust bridge returns no embeddings. It returns only speaker ID, times, and quality. | Feasible, must remain enforced |
| Concurrent dictation unaffected | The offline FFI call has no cancellation or yield API. No concurrent ASR measurement has run yet. | Scheduling contract required |

The approximate DER is a smoke metric from `spikes/issue-540/evaluate.py`, not an upstream-equivalent DER implementation. It should not become a release threshold without comparison against FluidAudio's scorer.

## Exact dependency and API

Murmur pins `fluidaudio-rs` commit `2d1083314104c812944b5150866d1e334db8eed7` in `app/src-tauri/Cargo.toml:108-109`. That crate is version 0.14.1 and pins the Swift FluidAudio package to exact version 0.14.1. SwiftPM resolved that tag locally to `d302273d49ef4d8914b27f20d342be482e8810f1`.

Murmur disables default features and requests only `asr`. This does not remove diarization at this pin. The crate declares `asr`, `vad`, and `diarization` as empty Cargo features and does not guard the bridge code with them. The Rust API is present in the linked artifact.

The exact call path is:

1. `FluidAudio::init_diarization(threshold)` calls a synchronous C bridge.
2. Swift creates `OfflineDiarizerConfig`, sets `config.clustering.threshold`, creates `OfflineDiarizerManager`, and awaits `prepareModels()` behind a `DispatchSemaphore`.
3. `FluidAudio::diarize_file(path)` calls `OfflineDiarizerManager.process(url)` behind another semaphore.
4. Rust receives `speaker_id`, `start_time`, `end_time`, and `quality_score` for each turn.

The API accepts a whole file only. It exposes no sample-buffer entry point, custom model directory, timings, progress, cancellation, or pause. It does not return FluidAudio's internal `speakerDatabase` embeddings. Exact sources are `fluidaudio-rs/src/lib.rs:339-380`, `fluidaudio-rs/swift/FluidAudioBridge.swift:164-225`, and `fluidaudio-rs/swift/FluidAudioBridge.swift:999-1086` at commit `2d108331...`.

## Model set and trust boundary

FluidAudio 0.14.1 requests these files from `FluidInference/speaker-diarization-coreml` with the `offline` variant:

| Asset | Bytes |
|---|---:|
| `Segmentation.mlmodelc` | 6,006,888 |
| `FBank.mlmodelc` | 1,797,068 |
| `Embedding.mlmodelc` | 13,494,485 |
| `PldaRho.mlmodelc` | 211,560 |
| `plda-parameters.json` | 89,416 |
| Total | 21,599,417 |

The Hugging Face repository page reports 129 MB because it contains older and alternate bundles that this offline path does not load. The measured set above is the installed requirement.

Model revision `1ed7a662fdc7109e36d822db793ee6eebdaf8594` is public, ungated, and CC BY 4.0. FluidAudio's downloader resolves the mutable `main` branch. Production must not call it on an absent cache. Murmur should download an exact manifest into a sibling staging directory, validate every file's size and SHA-256, atomically publish it as `speaker-diarization-coreml`, and call FluidAudio only after that validation succeeds. Remove must unload or stop the owner before deleting this exact directory and partial staging data.

The current Core ML installer is ASR-specific. `app/src-tauri/src/transcriber/coreml.rs:14-21` hardcodes Parakeet bundles, and `app/src-tauri/src/transcriber/coreml.rs:176-213` always calls `init_asr`. The current `ModelRuntimeManager` catalog in `app/src-tauri/src/model_runtime.rs:107-192` represents selectable transcription backends. A diarizer is an optional auxiliary model, so adding it as another selectable transcription model would give the catalog the wrong meaning. Add an auxiliary-model catalog or a separate typed diarization model state with the same single-flight, validation, progress, and terminal-state rules.

## Fixture evidence

Murmur had no trustworthy multi-speaker audio fixture. Every tracked WAV under `bench/audio` is generated by `bench/make_audio.sh` with one macOS `say` voice. FluidAudio 0.14.1's `DiarizationTestFixtures.swift` creates alternating sine tones, not speech, and carries no speaker ground truth.

The staged acceptance fixture is:

- Audio: AMI corpus `ES2004a.Mix-Headset.wav`, 16 kHz mono PCM, 1,049.354688 s, 33,579,394 bytes.
- Ground truth: `only_words/rttms/test/ES2004a.rttm`, 260 turns and four speakers, plus the full-session UEM.
- License: the AMI corpus and annotations are CC BY 4.0.
- Audio source: `https://groups.inf.ed.ac.uk/ami/AMICorpusMirror/amicorpus/ES2004a/audio/ES2004a.Mix-Headset.wav`.
- Annotation revision: `pyannote/AMI-diarization-setup` commit `67c2d539286e89f68952d5dcf83912bd9f01dfae`.

FluidAudio 0.14.1 calls this input `AMI SDM`, but its downloader selects `Mix-Headset.wav`. The official pyannote setup calls `Array1-01.wav` the SDM signal. The staged file is therefore a clean headset mix, not a distant-room microphone. That is a reasonable first proxy for Murmur's digitally mixed System Audio channel, but results must retain the exact `Mix-Headset` name.

Data and content hashes are under `/tmp/murmur-backlog-20260907/540-spike-data`. `SHA256SUMS` verifies all 29 staged model, fixture, and result files.

## M4 measurement

Target: fleet `mac-mini`, Apple M4, 10 CPU cores, 24 GB RAM. This is the lower-memory Apple Silicon node in the current fleet and the right first gate. The M5 Pro 48 GB MacBook can be a secondary comparison, not the acceptance baseline.

Command shape: release arm64 spike binary, threshold 0.7, one initialized `FluidAudio` instance, two sequential full-file passes, `/usr/bin/time -l` around the process.

| Phase | Wall time | Throughput | Processing per audio hour | Speakers | Turns |
|---|---:|---:|---:|---:|---:|
| Model initialization | 8.973 s | n/a | n/a | n/a | n/a |
| First inference | 4.489 s | 233.75x real time | 15.40 s | 4 | 94 |
| Warm inference | 3.373 s | 311.13x real time | 11.57 s | 4 | 94 |

The process used 436.48 MiB peak RSS and reported a 720.06 MiB peak memory footprint. It recorded no swaps. Initialization plus two inferences took 17.09 s total wall time.

The evaluator found the same canonical labels and boundaries on both passes. Its best one-to-one mapping covered all four RTTM speakers. Approximate DER was 7.15%, split into 5.02% missed speech, 1.34% false alarm, and 0.79% speaker error.

## Phase 1 dependency

Issue #539 closed through merged PR #546, merge commit `53976984e0152933be0bcddf217b751c66a5aa74`, and that commit is an ancestor of current `main`. The current tree contains dual-channel capture, durable sessions and segments, recovery, review, export, and privacy filtering.

The original PR body left two gates unresolved: production-bundle dual-channel smoke was unchecked, and its final-head standard Murmur Bench result was inconclusive. Later meeting fixes have also changed capture and storage. #540 can build on the merged code, but its final verification must run a current production dual-channel smoke and the required Murmur Bench gate instead of treating #546's old evidence as current.

## Integration prerequisites

1. Preserve a bounded remote PCM source until diarization finishes. Today `retain_audio = false` deletes each chunk WAV immediately after its transcript commits. A post-session whole-file pass would have no audio. Keep an owner-only temporary remote file or durable spool index with a size or duration cap, crash recovery, session ownership, and deletion after success, failure, cancellation, or session deletion. This temporary retention needs explicit user disclosure even though it is not long-term retention.

2. Keep attribution conservative. Current meeting segments are VAD and ASR chunks, not guaranteed speaker turns, and every shipped backend reports no timestamps. Assign a remote speaker ID only when one confident diarized speaker covers the voiced portion of that chunk. If coverage is low, two speakers occur, overlap exists, or confidence is insufficient, leave the canonical channel as plain `Them`. Do not split text or choose the dominant speaker.

3. Add a schema migration from current meeting schema v3. Keep `speaker = me|them` as the immutable capture channel. Add a nullable session-scoped remote speaker ID with a check that `Me` can never carry it. Add a `(session_id, speaker_id)` label table with `ON DELETE CASCADE`, bounded labels, stable default numbering by first confident appearance, and one transaction for rename plus workspace refresh. Existing `meeting_reviews.me_label/them_label` remains the channel fallback and cannot represent several remote speakers by itself.

4. Keep raw diarization turns and embeddings out of durable storage unless the product needs the turns for re-evaluation. The bridge already drops embeddings. The minimum durable result is the assigned opaque speaker ID on an unambiguous transcript chunk and the session-local display label. Never put labels, IDs, audio paths, or embeddings in telemetry. The existing all-build meeting sanitizer and log shipper exclusion remain useful defense in depth.

5. Give diarization an explicit resource owner. `ModelRuntimeManager::with_ready_backend` holds one mutex across model loading and the complete transcription operation at `app/src-tauri/src/model_runtime.rs:690-703`. Holding it across full-session diarization would delay dictation by about 12 to 15 seconds per audio hour on the measured M4. A separate in-process engine cannot be cancelled and may contend for the ANE. The safer current fit is a managed signed child with immutable session and generation, parent-death ownership, deadline, content-free protocol, and dictation-priority cancellation. A cancelled pass can restart when Murmur is idle. Validate this choice with a bounded concurrent Core ML ASR comparison before integration.

6. Fail open at every boundary. Missing model means no pass starts. Model load, worker exit, deadline, low confidence, incomplete audio, or store conflict leaves channel attribution and text untouched. Commit all accepted speaker assignments and the label rows together only after a complete pass validates against the same session generation.

Reuse the existing ownership code instead of creating a second process system. `app/src-tauri/src/managed_child.rs` owns child process groups and termination. `app/src-tauri/src/coreml_installer.rs` shows the same-signed worker protocol and deadline pattern. `app/src-tauri/src/meeting_capture.rs` already carries meeting generations, spools WAVs, and rejects stale continuation work. `app/src-tauri/src/capture_helper_probe.rs` has confirmed-termination evidence. `app/src-tauri/src/llm_sidecar.rs` and `app/src-tauri/src/commands/meeting_summary.rs` show child RSS sampling and cancellable status publication. The new worker protocol should carry no audio paths supplied by the frontend and no text, labels, or embeddings.

## Reproduction commands

Build the isolated spike with the same pin and linker settings as Murmur:

```bash
cd /Users/george-mac-mini/Documents/code/murmur-app-issue-540/spikes/issue-540
cargo build -j 2 --release
```

Run the measured two-pass fixture:

```bash
mkdir -p /tmp/murmur-backlog-20260907/540-spike-data/results
/usr/bin/time -l \
  target/release/murmur-issue-540-diarization-spike \
  /tmp/murmur-backlog-20260907/540-spike-data/fixture/ES2004a/ES2004a.Mix-Headset.wav \
  0.7 2 \
  > /tmp/murmur-backlog-20260907/540-spike-data/results/es2004a-threshold-0.7.jsonl \
  2> /tmp/murmur-backlog-20260907/540-spike-data/results/es2004a-threshold-0.7.time.txt
```

Score and check repeat stability:

```bash
python3 evaluate.py \
  /tmp/murmur-backlog-20260907/540-spike-data/results/es2004a-threshold-0.7.jsonl \
  /tmp/murmur-backlog-20260907/540-spike-data/fixture/ES2004a/ES2004a.rttm \
  /tmp/murmur-backlog-20260907/540-spike-data/fixture/ES2004a/ES2004a.uem \
  --audio-seconds 1049.354688
```

Regenerate the content manifest after adding results:

```bash
spike_root=/tmp/murmur-backlog-20260907/540-spike-data
manifest_tmp=/tmp/murmur-backlog-20260907/540-spike-data.sha256.tmp
find "$spike_root" -type f ! -name SHA256SUMS -print0 \
  | sort -z | xargs -0 shasum -a 256 > "$manifest_tmp"
mv "$manifest_tmp" "$spike_root/SHA256SUMS"
shasum -a 256 -c "$spike_root/SHA256SUMS"
```

`run_concurrent_probe.sh` and `src/bin/asr_probe.rs` prepare the next comparison. They have not been executed. The script measures Core ML ASR alone and in a separate process while full-session diarization runs. Run it only in an exclusive performance slot. Its separate processes approximate the accepted managed-child design better than two engines sharing one Rust object.

## Autonomous next steps

1. Run the prepared concurrent ASR probe in an exclusive measurement slot. Compare warm Core ML dictation latency and process RSS alone versus during a diarization pass.
2. Freeze the exact model manifest and CC BY attribution in the implementation plan. Do not depend on Hugging Face `main` at runtime.
3. Implement the auxiliary installer/status/remove contract and managed diarization worker before meeting UI work.
4. Add bounded remote PCM staging and crash cleanup, then the schema migration and conservative assignment transaction.
5. Add tests for absent model, worker failure, ambiguous chunks, mic exclusion, rename cascade, cross-session isolation, telemetry stripping, and transcript/order identity.
6. Verify the final app with this pinned AMI fixture, a current production dual-channel smoke, concurrent dictation latency, and Murmur Bench `quick` on the immutable PR head.

Prepared spike worktree: `/Users/george-mac-mini/Documents/code/murmur-app-issue-540`, branch `issue/540-diarization-spike`. The spike crate and evaluator are uncommitted and separate from production integration.
