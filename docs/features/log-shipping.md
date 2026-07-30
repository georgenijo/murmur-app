# Diagnostic Log Shipping

Zero-config telemetry upload. Every install tails its own structured event log
(`events.jsonl`, see [log-viewer.md](log-viewer.md)) and ships new lines to a
central ingest endpoint. No user setup, no UI, no permissions prompts —
install the app (or receive an auto-update) and logs flow.

## Privacy model

- Only the **structured event stream** leaves the device — the same
  privacy-stripped events shown in the in-app log viewer. Transcription text
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
- Dev builds ship `events.dev.jsonl` with `X-Dev: 1`; the receiver stores them
  as `events.dev.jsonl` so dev noise is separable.
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
and freshness; click a row for the per-device page (recent warnings/errors on
top, last 200 events below). Served by the same receiver process.

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
