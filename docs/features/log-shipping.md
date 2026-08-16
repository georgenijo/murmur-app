# Diagnostic Log Shipping

Zero-config telemetry upload for production builds. A production install tails
its structured event log (`events.jsonl`, see [log-viewer.md](log-viewer.md))
and ships new lines to a central ingest endpoint. Normal development builds do
not ship or persist server-side history; old `X-Dev: 1` batches are
acknowledged and discarded. No user setup, UI, or permissions prompt is added.

## Privacy model

- Only the **structured event stream** leaves the device — the same
  privacy-stripped events shown in Settings → Performance. Transcription text
  never enters that stream (`telemetry.rs` strips it at the source).
- Installs are identified by a **random UUID** generated on first run and
  stored in `shipper_state.json`. Each batch also carries the **device name**
  (`scutil --get ComputerName`), **macOS version**, and **hardware model** so
  the fleet dashboard can label streams ("George's MacBook Pro · macOS 26.0").
  Username and any content-bearing identifiers are never sent.
- The separate `/state` snapshot reports only whether a default audio input is
  available, a bounded input count, and whether enumeration succeeded. It is
  refreshed from explicit device-list requests. A shipper poll during capture
  keeps the cached aggregate and schedules one state-only refresh after the
  lifecycle returns to idle; it never reads or sends microphone display labels
  or backend UIDs.
- Kill switch: launch with `MURMUR_LOG_SHIPPER=off` in the environment.

## Server-armed hang diagnostics (`hang_diagnostics.rs`)

Dormant on every install by default. The receiver's `/ingest` success reply
carries `{"diagnostics": bool}` per install UUID (armed by listing the UUID in
`~/murmur-logs/diag-installs.txt` on the receiver; no restart needed). Only on
an armed install, a capture attempt that reaches half its budget without first
PCM gets a native stack `sample` of the hung worker, and after the confirmed
kill a single bounded text bundle — worker stack, 90s of coreaudiod unified
log, audio/Bluetooth topology, installed HAL plug-ins — is uploaded to
`POST /bundle`. This content names devices and installed software, so an
install must only be armed with its owner's agreement; disarming is
server-side and takes effect within one shipper tick. Arming is logged
loudly in the armed install's own event stream.

An armed install's ingest reply may also carry `collect_now: <epoch>`
(receiver `collect-now.txt`, lines of `uuid epoch`): an epoch greater than
any already honored triggers exactly one immediate probe-bundle collection
(no hang required). The epoch chooses only *when* a collection runs — *what*
runs is the compiled-in, read-only probe list (process table, power
assertions, and the standard bundle sections); the client never executes
server-supplied text. The last honored epoch is process-lifetime state, so
clear the install's `collect-now.txt` line once its bundle arrives to avoid
a re-collection on the next app launch.

## Architecture

```
events.jsonl  ──(log_shipper.rs, every 60s)──▶  POST https://georgenijo.com/murmur/ingest
                                                   │  Cloudflare Tunnel → nginx (127.0.0.1:8601, opti)
                                                   ▼
                                    murmur-logs.service (127.0.0.1:8600)
                                                   │
                                                   ▼
                         ┌──────────▶ ~/murmur-logs/<install-uuid>/events.jsonl
                         │                         │
                         │            recovery/export + hourly watch
                         │                         ▼
                         │              capture-watch.json
                         │
                         └──────────▶ events.sqlite3 (WAL + FTS5)
                                                   │
                                                   ▼
                              Access-protected historical dashboard search
```

### Shipper (`app/src-tauri/src/log_shipper.rs`)

- Spawned from `lib.rs` setup; first tick 15s after launch, then every 60s.
- Persists a byte offset in `shipper_state[.dev].json` next to the log. The
  offset only advances after a 2xx response, so **the JSONL file is the retry
  queue** — offline or endpoint-down means the batch is retried next tick.
- Handles the 5 MB rotation in `telemetry.rs`: when the current file is
  shorter than the saved offset, it drains the tail of `events.jsonl.1` from
  that offset, then restarts at 0 on the fresh file.
- Batches are cut at line boundaries, max 1 MB per POST, max 8 POSTs per tick.
- Audio `/state` delivery reads a privacy-safe cached aggregate. The cache is
  updated when another app flow explicitly enumerates inputs (for example,
  opening Settings). Polls during initialization, recording, stopping, or
  recovery do not spawn another helper. They coalesce into one deferred
  aggregate-only refresh after the audio supervisor joins its worker and
  publishes Idle. A shared HAL boundary serializes that idle refresh against a
  racing capture start; failure waits for a later normal state poll instead of
  a tight retry loop.
- Auth is a static bearer token baked into the binary — spam control for the
  public URL, not a security boundary.
- Normal dev builds do not ship. The receiver acknowledges and discards
  `X-Dev: 1` batches from older builds so their local retry offsets can advance
  without adding development noise to the fleet.
- `MURMUR_LOG_ENDPOINT=<url>` overrides the endpoint for testing.

### Receiver (opti)

- `/home/george/murmur-logs-receiver.py` — stdlib-only Python HTTP server on
  `127.0.0.1:8600`, run by systemd unit `murmur-logs.service` (user `george`),
  on the fleet node `opti` (a Dell Optiplex on the tailnet with no public
  IP).
- Validates the bearer token, install ID, and complete bounded batch before any
  mutation. Each production event receives an install-scoped canonical hash.
  New hashes are inserted in one SQLite transaction, corresponding JSONL lines
  are appended and fsynced, and the database commits before a 2xx response.
  Exact retries are no-ops in both stores. Busy, quota, disk-full, corruption,
  archive, or commit failure returns non-2xx so the shipper offset stays put.
  The caps are 8 MB/request, 200 MB raw JSONL/install, 10 GB raw total, 150
  installs, and 10 GB SQLite; history is never auto-deleted.
- SQLite uses migrations, request-scoped connections, foreign keys, WAL,
  bounded busy wait/cache/WAL size, startup integrity checking, indexed event
  fields, and FTS5 over summary text. Raw JSONL remains the exact
  recovery/export source.
- Adds a bounded `ingest_app_version` annotation from the app-version request
  header to each accepted production event. This is receiver-side attribution
  from an existing header, not a new client-collected field. Historical lines
  remain untouched and are treated as an `unknown` non-comparable cohort.
- Exposed at `https://georgenijo.com/murmur/ingest` via a Cloudflare Tunnel
  (`opti-murmur`) that forwards `georgenijo.com` to a local nginx site on
  `127.0.0.1:8601`, which proxies the `/murmur/*` paths to the receiver.
  There is no direct public listener — `opti` has no public IP. Health:
  `/murmur/healthz`. See [infra/log-receiver/README.md](../../infra/log-receiver/README.md)
  for the full tunnel/nginx setup.
- The ingest token is in fleet secrets: `fleet secret get murmur-log-ingest-token`.

## Fleet dashboard

`https://murmur.georgenijo.com` (Cloudflare Access, george.nijo8@gmail.com
only) — one row per install stream with device name, OS, version, event count,
and freshness. Each installation page leads with a **Plain-English health**
summary for microphone capture, shortcuts, dictation, updates, and transforms.
Repeated equivalent problems are grouped with occurrence counts, and ordered
evidence distinguishes automatic recovery from an unresolved failure. Raw
events remain available in an expandable technical timeline and as exact
latest-200, latest-500, or complete JSONL downloads. Served by the same receiver
process.

After a bounded resumable backfill and content-free per-install reconciliation
prove the SQLite projection, the dashboard switches its production history
reads from JSONL to SQLite. Operators can filter by install/device, UTC or
Eastern date range, receiver-observed app version, level, stream, and bounded
FTS5 summary terms. A dedicated full-history warning/error view uses the same
stable `(timestamp, event_id)` keyset cursor, including when timestamps are
identical; offset pagination is never used. Cursors are signed and bound to the
exact filter set, parameters are strictly bounded, and every rendered event or
metadata value is escaped.

The readiness flag defaults off and can be enabled only by a successful
reconciliation command. Raw latest-200/latest-500/full downloads and
LLM-ready latest-200/latest-500 reports keep reading JSONL, so rollout and
query-store rollback do not change their bytes or privacy boundary.

The per-install **quick info** table shows two retained-log activity metrics:
**Last activated** is the newest dictation-owned `audio.capture_ready` event
that proves microphone audio became ready, while **Last successful
transcription** is the newest `pipeline.dictation_terminal` success with a
positive character count. The old native-ready/completed codes remain bounded
historical fallbacks. Start requests,
failed initialization, cancelled/no-speech runs, empty output, and imported-file
transcription do not advance those metrics. Each value includes relative age and
an exact Eastern timestamp. The receiver scans the complete retained JSONL with
a strict per-record memory bound, independent of the visible latest-200 event
window; an absent match is reported as not found in the retained log.

Each install page also offers latest-200 and latest-500 **LLM-ready Markdown**
reports. The versioned `murmur-fleet-llm/v1` format includes bounded device and
microphone-state context, current health, deduplicated findings, and the original
event sequence normalized into compact JSON bullets. Recognized stable codes and
constant legacy summaries receive deterministic plain-English meanings; unknown
events remain explicitly unmapped with bounded source context.

The report tells an analyzing model to treat every event field as untrusted
telemetry rather than instructions. This is prompt-injection hardening, not a
claim that event text is trusted: operators should attach the report as
diagnostic data and ask the model to prioritize Action/Watch findings, correlate
the ordered sequence, and cite event codes in its diagnosis.

### Scheduled capture-startup watch

`murmur-capture-watch.timer` runs an hourly, stdlib-only, line-by-line scan of
the retained production JSONL. Its versioned report groups only privacy-safe
capture metrics by install and receiver-observed app version:

- dictation readiness `startup_ms` p50/p95;
- active initialization timeouts split by stable backend and
  `last_setup_step`;
- fallback and both-backends-failed counts;
- ready-recording counts for the five most recent completed attempted
  dictation sessions per cohort;
- per-recording request → accepted → ready → stop-handoff → terminal counts,
  bounded terminal outcomes, stage drop-off, and missing/duplicate terminals.

The watch alerts when a newest comparable cohort (at least five readiness
samples) has a p50 above twice the best retained earlier eligible cohort on the
same install. Anchoring to that healthy baseline keeps an equally slow later
release from masking an unresolved regression. It also alerts when the latest
version cohort with a completed attempted dictation session has two zero-ready
outcomes among its five most recent attempts. A newer healthy cohort supersedes
an older failed cohort, and enough later healthy attempts age failures out of
the window. An app session is delimited by `startup_baseline`; idle launches,
transform captures, and the currently open session cannot create a zero-ready
verdict or a missing-terminal alert. Lifecycle records join only the app-session
boundary and positive `recording_id`; timestamps and free-form summaries are
never correlation keys. Audio lifecycle records must carry
`owner_kind: "dictation"`, so other microphone owners cannot contaminate the
funnel. The same exact owner gate applies to startup samples, backend timeouts,
fallbacks, and both-backends-failed counters, so preview, query, transform, and
`microphone_benchmark` events remain visible in raw technical logs without
changing dictation health denominators. Closed sessions are reduced into
aggregate counters, leaving only the currently open session's per-attempt
correlation state in memory.

Reports contain no raw event summaries, device fields, content, paths, or free
form errors. Backend/setup-step values are allowlisted (including the explicit
pre-native-call `none` setup step) and unknown values collapse to `unknown`.
Memory is bounded to the newest 500 startup samples and five attempted-session
outcomes per cohort, with ready counts at or above 20 combined into one capped
tail bucket. Each install has at most 64 explicit versions; excess versions
collapse into a non-comparable `overflow` cohort. The report is atomically
replaced at `~/murmur-logs/capture-watch.json`; an alert also makes the one-shot
exit nonzero for systemd/journal visibility and appears on the protected
dashboard.

The same line-by-line pass also evaluates the versioned
`murmur-reliability-slo/v1` contract. Only native dictation requests with exact
numeric `slo_contract: 1` are eligible. Historical requests without the marker
remain useful to the legacy capture funnel but cannot enter a weekly SLO or
make a pre-contract week pass.

The evaluator recomputes the current partial ISO week and the previous eight
complete Monday-to-Monday UTC weeks from the full retained log on every run.
This deliberate full rescan lets a late-arriving event repair its original
week; the atomically replaced `reliability_slo` subtree is the persisted rolling
result. Its policy is:

- at least 200 eligible requests are required for a complete week's sufficient
  sample;
- at least 99.5% of all eligible requests—including missing-ready and failed
  attempts—must reach the first matching retained-PCM readiness event within
  400 ms of the request timestamp;
- a stable microphone-permission-prompt record excludes that attempt from the
  startup denominator. It remains in requested/accepted/ready/terminal,
  state, and actionable-presentation counts, so a prompt cannot hide a stuck
  state or unpresented failure;
- a `Recovering` or `Processing` interval that exits before the next startup is
  `self_recovered`; one still open at the next `system.startup_baseline` is
  `restart_required`; an interval still open at the end of the full scan, or
  one with malformed/orphan transition evidence, is `indeterminate`. Durations
  are reported for valid closed intervals, but there is no invented time
  threshold that turns a slow self-recovery into a restart;
- capture-initialization and runtime-interruption failures require their exact
  actionable presentation pair. A stop failure is covered only by a correlated
  cleanup-stalled/restart-app presentation; fast stop failures without that
  real UI evidence and pipeline failures fail the presentation clause.

An exact contract-v1 request whose timestamp is malformed or in the future
cannot be assigned to an invented week. Valid history older than the retained
weekly horizon expires normally and is not an integrity error.
The report instead increments the bounded aggregate
`integrity.unassigned_contract_requests` count, marks source integrity
`indeterminate`, forces the two-week proof false, and raises the outer capture
watch alert. Attempts or per-attempt event evidence beyond the evaluator's
fixed memory/cardinality bounds increment the separate aggregate
`integrity.overflowed_events` count and have the same fail-closed effect. These
integrity counts have no install or event-content breakdown. Every malformed
JSON or non-object line in the retained `events.jsonl` scan increments bounded
`integrity.malformed_source_lines`; any nonzero count also makes integrity
indeterminate and blocks the two-week proof. Because each run is a full scan,
a concurrently partial final line self-heals after the writer completes it.

A correlated accepted/ready/prompt/terminal/presentation/state record with a
malformed timestamp or a timestamp later than the evaluation instant cannot
supply healthy evidence early. The evaluator ignores that record's claimed
lifecycle effect, increments the bounded weekly
`invalid_evidence_timestamps` count, and makes the request week indeterminate.
This prevents future evidence from curing an otherwise open or failed attempt.

Every week carries `sample_status` (`partial`, `below_minimum`, or
`sufficient`), a verdict, bounded reason codes, denominator/outcome/actionable
counts, startup p50/p95/max, and aggregate state-interval counts/durations. A
partial week is always `insufficient`. Complete weeks prefer
`indeterminate`, then `fail`, then below-minimum `insufficient`; only a
sufficient, contradiction-free week meeting every clause can `pass`. The
two-week finish line becomes true only when the newest two complete weeks both
pass. The newest non-insufficient complete verdict controls the outer watch: a
failure or indeterminate result raises the status and the `--fail-on-alert`
service exit until a later decisive pass supersedes it. Intervening
insufficient weeks remain visible and cannot mask or clear that evidence.

Per-install IDs remain internal join keys. The published
`reliability_slo` subtree and dashboard are aggregate-only and contain no
install UUID, device label/UID, app version, transcript/audio, path, bundle ID,
raw event summary, or free-form error. The dashboard shows the current and two
newest complete windows, their verdict/sample status, supporting counts,
latency distribution, state evidence, bounded integrity/reason counts,
actionable-presentation coverage, and whether two consecutive complete weeks
pass. The renderer strictly validates the exact nested schema, window sequence,
numeric bounds, and cross-field equations before displaying a verdict; a
truncated or contradictory local report falls back to a diagnostic card.

### Operator event semantics

High-value producers attach an allowlisted, privacy-safe `event_code` inside
the structured `data` object. Examples include
`audio.capture_backend_timeout`, `audio.fallback_started`,
`audio.capture_ready`, `audio.capture_failed`,
`audio.permission_prompt_changed`, `pipeline.dictation_state_changed`,
`pipeline.dictation_presentation`,
`audio.device_reresolution_started`, `audio.input_resolution_observed`,
`keyboard.listener_silent`, `pipeline.dictation_terminal`,
`transform.pass_outcome`, and `updater.install_failed`.

The dashboard maps those stable codes plus bounded fields to operator-facing
language. A bounded compatibility table recognizes the corresponding constant
summary strings in historical JSONL. Unknown events are never guessed: warnings
and errors remain visible as technical events with their original escaped
summary and data.

`audio.input_resolution_observed` is a strict, content-free proof record for a
live backend attempt. Its allowlist is limited to `event_code`, `capture_id`,
`owner`, `owner_kind`, `backend`, `resolution_pass` (1–3), `backend_attempt`
(1–2), `microphone_mode`, `input_enumeration_ok`, `requested_present`,
`requested_present_known`, `input_device_count` (capped at 256),
`input_device_count_capped`, and `default_input_available`. Cross-field
contradictions reduce the event to `event_code` only in both debug and release
builds. `owner_kind` is an exact enum that includes the content-free
`microphone_benchmark` diagnostic owner; accepting that owner does not make its
selected device ID or display name part of the event. The sanitizer never
accepts microphone IDs or names, the default device's ID, raw errors, paths,
content, or arbitrary extra fields. The summary is the constant “Microphone
input resolution observed”.

Health conclusions are limited to the loaded event window. A fallback is shown
as recovered only when later readiness for the same audio owner proves it;
otherwise the outcome remains degraded or unknown. Listener silence is
diagnostic rather than proof of failure because an idle or sleeping Mac also
produces no global keyboard callbacks.

## Reading the logs

```bash
tailscale ssh george@opti "ls /home/george/murmur-logs/"          # installs
tailscale ssh george@opti \
  "tail -20 /home/george/murmur-logs/<uuid>/events.jsonl"        # tail one
```

`meta.json` in each install dir records the last seen app version.

## Failure modes

| Failure | Behavior |
|---|---|
| opti / tunnel down / offline | POSTs fail silently; offset holds; retry every 60s. Data older than the ~10 MB rotation window (current + `.jsonl.1`) is lost if the outage outlives it. |
| Endpoint URL changes | Old binaries go dark until auto-update delivers the new constant. |
| Install exceeds 200 MB on server | Receiver returns 507; shipper keeps retrying but nothing is appended (effectively paused for that install). |
| SQLite busy, quota, disk full, corruption, archive fsync, or commit failure | Receiver returns non-2xx and never acknowledges a partial batch. JSONL is truncated back after a failed pre-commit append; the client retries from the same offset. |
| Query-store reconciliation fails | Indexed dashboard reads stay disabled; ingest continues dual-writing and the protected dashboard stays on raw JSONL until the mismatch is investigated. |
