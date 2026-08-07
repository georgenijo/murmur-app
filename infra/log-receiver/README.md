# Log receiver deployment (whoop-vm)

Server side of [docs/features/log-shipping.md](../../docs/features/log-shipping.md).
Everything here is a snapshot of what runs on whoop-vm — treat this directory as
the source of truth and redeploy from it.

## Components

| File | Deploys to | Purpose |
|---|---|---|
| `murmur-logs-receiver.py` | `/home/george/murmur-logs-receiver.py` | stdlib HTTP server on `127.0.0.1:8600`: ingest, state, dashboard |
| `murmur-logs.service` | `/etc/systemd/system/` | runs the receiver as `george`, restart-always |
| `murmur-capture-watch.py` | `/home/george/murmur-capture-watch.py` | bounded stdlib aggregation of shipped capture-startup metrics |
| `murmur-capture-watch.service` | `/etc/systemd/system/` | one-shot capture regression analysis |
| `murmur-capture-watch.timer` | `/etc/systemd/system/` | runs the watch hourly with a randomized delay |
| `nginx-murmur-site.conf` | `/etc/nginx/sites-available/murmur` (+ symlink in sites-enabled) | `murmur.georgenijo.com` → dashboard (own LE cert) |
| `nginx-georgenijo-locations.snippet` | inside the `georgenijo.com` 443 server block | public `/murmur/ingest`, `/murmur/state`, `/murmur/healthz` |

## Also required (not files)

- **DNS**: `murmur.georgenijo.com` A → VM, Cloudflare-proxied. `georgenijo.com` stays DNS-only.
- **Cloudflare Access** app "Murmur Fleet Logs" gating host `murmur.georgenijo.com`,
  allow policy for george.nijo8@gmail.com. (CF API token: keychain `cloudflare-api-token-whoop`.)
- **TLS**: `certbot --nginx -d murmur.georgenijo.com` (issue while the DNS record is
  un-proxied, then flip to proxied). Auto-renews.
- **Ingest token**: baked into the receiver and `app/src-tauri/src/log_shipper.rs`;
  reference copy in fleet secrets: `fleet secret get murmur-log-ingest-token`.

## Redeploy after editing the receiver

```bash
cat infra/log-receiver/murmur-logs-receiver.py | tailscale ssh george@whoop-vm "cat > /home/george/murmur-logs-receiver.py"
cat infra/log-receiver/murmur-capture-watch.py | tailscale ssh george@whoop-vm "cat > /home/george/murmur-capture-watch.py"
cat infra/log-receiver/murmur-capture-watch.service | tailscale ssh george@whoop-vm "cat > /tmp/murmur-capture-watch.service"
cat infra/log-receiver/murmur-capture-watch.timer | tailscale ssh george@whoop-vm "cat > /tmp/murmur-capture-watch.timer"
tailscale ssh ubuntu@whoop-vm "sudo install -m 0644 /tmp/murmur-capture-watch.service /etc/systemd/system/ && sudo install -m 0644 /tmp/murmur-capture-watch.timer /etc/systemd/system/ && sudo systemctl daemon-reload && sudo systemctl enable --now murmur-capture-watch.timer"
tailscale ssh ubuntu@whoop-vm "sudo systemctl restart murmur-logs && systemctl is-active murmur-logs"
tailscale ssh ubuntu@whoop-vm "sudo systemctl start murmur-capture-watch.service || true; systemctl status --no-pager murmur-capture-watch.service; systemctl list-timers --no-pager murmur-capture-watch.timer"
curl -s https://georgenijo.com/murmur/healthz   # expect: ok
```

The one-shot exits `2` after atomically writing its report when it finds an
alert. That intentionally leaves an operator-visible failed unit and journal
entry; the timer continues to run on schedule. The fleet dashboard reads the
same report and shows the bounded alert details. A healthy later run clears the
failed state and replaces the report.

Rollback stops/disables the timer, removes its two units and script, reloads
systemd, and redeploys the preceding receiver. Retained `events.jsonl` remains
valid: `ingest_app_version` is an additive receiver annotation.

## Data layout

`/home/george/murmur-logs/<install-uuid>/` — `events[.dev].jsonl` (append-only),
`meta.json` (device identity), `state.json` (latest mic/device snapshot).
Caps: 8 MB/request, 200 MB/install, 10 GB global, 150 installs.
Dev-tagged batches are acked and discarded.

Accepted production events are annotated with the bounded
`ingest_app_version` already supplied in the request header. This adds no
client-collected field. Pre-deployment retained lines remain unmodified and the
watch groups them under `unknown`, which is never used for version comparisons.

`capture-watch.json` is a versioned, atomically replaced derived report. It
contains no source event text or device metadata: only install UUID, bounded app
version/backend/setup-step labels, counts, and startup durations.

## Capture regression watch

The hourly watch scans production JSONL line-by-line and keeps at most the
newest 500 readiness samples per install/version cohort and 64 explicit version
cohorts per install. Further versions collapse into a non-comparable
`overflow` cohort. It reports:

- `startup_ms` p50 and p95;
- active-budget timeouts split by stable backend and native setup step;
- fallback and both-backends-failed counts;
- a histogram of ready recordings per completed, attempted app session.

`startup_baseline` begins an app session. A session contributes to the
zero-ready signal only after a later baseline proves it ended and only when it
contained `audio initialization accepted`; idle launches and the currently open
session are excluded.

An alert is created when the newest comparable version has at least five
readiness samples and its p50 is more than twice the preceding comparable
version on the same install, or when one install/version has at least two
completed attempted sessions with zero ready recordings. Thresholds live in the
watch script and are fixture-tested.

## Tests

The receiver stays stdlib-only. Its plain-English classification, recovery
correlation, grouping, unknown-event fallback, exact newest-N downloads,
LLM-ready Markdown rendering, bounded retained-log activity metrics, route
bounds, and HTML escaping are covered by:

```bash
python3 -m unittest tests/test_log_receiver.py
python3 -m unittest tests/test_capture_regression_watch.py
```

Install pages expose raw JSONL downloads for the latest 200, latest 500, or the
complete stream. The LLM-ready latest-200/latest-500 downloads use the versioned
`murmur-fleet-llm/v1` Markdown format documented in
[`docs/features/log-shipping.md`](../../docs/features/log-shipping.md).
Quick info also reports the latest proven microphone activation and latest
non-empty live transcription across the complete retained stream.
