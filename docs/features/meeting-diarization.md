# Local meeting speakers

Remote speaker labels are an optional post-meeting refinement. Install the
21.6 MB speaker model set, then enable remote speaker labels before starting a
meeting. Microphone text keeps its existing Me channel. The model processes
only system playback, entirely on this Mac.

Murmur temporarily saves the remote audio while the meeting runs and speaker
labels finish. This temporary file is separate from Keep meeting audio. It is
private to your user, capped at two hours, and deleted after success, failure,
cancellation, supersession, or session deletion. On restart Murmur removes any
abandoned temporary files. Exceeding two hours abandons speaker refinement for
that meeting; capture and transcription continue normally.

The live transcript initially shows Them. After the transcript finishes, the
worker identifies voices within this meeting and assigns Speaker 1, Speaker 2,
and so on to sufficiently clear passages. Rename a speaker in the meeting
workspace to update all their assigned passages. Names belong to that meeting
and are included in its exports. Names and speaker assignments disappear when
the meeting is deleted.

One transcription chunk may contain several speakers. Murmur preserves its text
and leaves it as Them when it cannot attribute the whole passage conservatively.
It never guesses how to divide text or chooses the loudest voice. Overlap,
uncertain model results, insufficient coverage, missing models, and worker
failures keep the original channel attribution.

## Resource ownership

The already-signed app executable runs a dedicated worker process. It verifies
every model file against a pinned SHA-256 manifest and denies network access
and writes to the pinned model directory before entering FluidAudio. The worker returns only bounded turn times,
session-local integers and quality scores. Embeddings stay in the worker's
memory. Neither turns, audio paths, speaker names, meeting titles, attendees, nor
embeddings enter logs.

A pass starts only while the app is idle. Dictation takes priority: before
capture or ASR begins, Murmur kills the exact child process group and confirms
termination with a 150 ms limit. If cleanup is unconfirmed, that owner remains
tracked and the foreground operation asks for a short retry. A cancelled pass
may restart when idle, at most eight times within ten minutes. Each worker pass
has a two-minute deadline. No diarization call holds the ASR runtime mutex.

## Model attribution

The offline models are by FluidInference, distributed under CC BY 4.0:
<https://huggingface.co/FluidInference/speaker-diarization-coreml>.
Murmur pins revision `1ed7a662fdc7109e36d822db793ee6eebdaf8594` and does not change
the model files. Required bundles are Segmentation, FBank, Embedding, PldaRho,
and plda-parameters.json, totaling 21,599,417 bytes. Model removal stops any
active speaker worker before deleting this auxiliary model set.

## Verification

The measured prerequisite and exact AMI fixture provenance are recorded in
[the spike](../design/issue-540-spike.md). Run
`scripts/verify_meeting_diarization.py` with the final signed app executable and
explicit local fixture to verify repeated speaker labels, boundaries and
parent-death cleanup. Its report contains content-free hashes and counts.

Rust tests cover conservative attribution, microphone exclusion, raw transcript
identity, bounded private audio cleanup, strict worker results and exact child
preemption. Storage tests cover the v4 speaker migration, the v5 meeting metadata
migration, transactional assignment, renaming, session isolation, export labels,
and cascading deletion. Native
capture smoke remains a separate check.

The debug test bundle also supports two opt-in proof paths through that script:
`--asr-fixture` measures three paired warmed Core ML decodes around real worker
preemption using the production ASR runtime and foreground reservation.
`--seed-gui` runs the actual worker and production assignment/store methods to
create two meetings in the empty `com.localdictation.diarization540` test store.
It refuses nonempty stores and accepts only the pinned public AMI fixture hash.
The visible transcript uses explicit fixture markers, not recognized speech.
Use those meetings for native rename, session-isolation and export checks.
