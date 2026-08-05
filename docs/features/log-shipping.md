# Diagnostic Log Shipping

Zero-config telemetry upload. Every install tails its own structured event log
(`events.jsonl`, see [log-viewer.md](log-viewer.md)) and ships new lines to a
central ingest endpoint. No user setup, no UI, no permissions prompts —
install the app (or receive an auto-update) and logs flow.

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
  available, a bounded input count, and whether enumeration succeeded. It
  never reads or sends microphone display labels or backend UIDs.
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
                                                   │  nginx (georgenijo.com site, whoop-vm)
                                                   ▼
                                    murmur-logs.service (127.0.0.1:8600)
                                                   │
                                                   ▼
                                    ~/murmur-logs/<install-uuid>/events[.dev].jsonl
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
- Auth is a static bearer token baked into the binary — spam control for the
  public URL, not a security boundary.
- Normal dev builds do not ship. The receiver acknowledges and discards
  `X-Dev: 1` batches from older builds so their local retry offsets can advance
  without adding development noise to the fleet.
- `MURMUR_LOG_ENDPOINT=<url>` overrides the endpoint for testing.

### Receiver (whoop-vm)

- `/home/george/murmur-logs-receiver.py` — stdlib-only Python HTTP server on
  `127.0.0.1:8600`, run by systemd unit `murmur-logs.service` (user `george`).
- Validates the bearer token, the `X-Install-Id` shape, and that every line
  parses as JSON before appending. 8 MB request cap, 200 MB per-install cap.
- Exposed at `https://georgenijo.com/murmur/ingest` via a `location` block in
  `/etc/nginx/sites-enabled/georgenijo.com`. Health: `/murmur/healthz`.
- The ingest token is in fleet secrets: `fleet secret get murmur-log-ingest-token`.

## Fleet dashboard

`https://murmur.georgenijo.com` (Cloudflare Access, george.nijo8@gmail.com
only) — one row per install stream with device name, OS, version, event count,
and freshness. Each installation page leads with a **Plain-English health**
summary for microphone capture, shortcuts, dictation, updates, and transforms.
Repeated equivalent problems are grouped with occurrence counts, and ordered
evidence distinguishes automatic recovery from an unresolved failure. Raw
events remain available in an expandable technical timeline and as a complete
JSONL download. Served by the same receiver process.

### Operator event semantics

High-value producers attach an allowlisted, privacy-safe `event_code` inside
the structured `data` object. Examples include
`audio.capture_backend_timeout`, `audio.fallback_started`,
`audio.capture_ready`, `audio.capture_failed`,
`keyboard.listener_silent`, `pipeline.dictation_completed`,
`transform.pass_outcome`, and `updater.install_failed`.

The dashboard maps those stable codes plus bounded fields to operator-facing
language. A bounded compatibility table recognizes the corresponding constant
summary strings in historical JSONL. Unknown events are never guessed: warnings
and errors remain visible as technical events with their original escaped
summary and data.

Health conclusions are limited to the loaded event window. A fallback is shown
as recovered only when later readiness for the same audio owner proves it;
otherwise the outcome remains degraded or unknown. Listener silence is
diagnostic rather than proof of failure because an idle or sleeping Mac also
produces no global keyboard callbacks.

## Reading the logs

```bash
tailscale ssh george@whoop-vm "ls /home/george/murmur-logs/"          # installs
tailscale ssh george@whoop-vm \
  "tail -20 /home/george/murmur-logs/<uuid>/events.jsonl"             # tail one
```

`meta.json` in each install dir records the last seen app version.

## Failure modes

| Failure | Behavior |
|---|---|
| whoop-vm down / offline | POSTs fail silently; offset holds; retry every 60s. Data older than the ~10 MB rotation window (current + `.jsonl.1`) is lost if the outage outlives it. |
| Endpoint URL changes | Old binaries go dark until auto-update delivers the new constant. |
| Install exceeds 200 MB on server | Receiver returns 507; shipper keeps retrying but nothing is appended (effectively paused for that install). |
