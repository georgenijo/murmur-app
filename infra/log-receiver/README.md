# Log receiver deployment (opti)

Server side of [docs/features/log-shipping.md](../../docs/features/log-shipping.md).
Everything here is a snapshot of what runs on `opti` (a Dell Optiplex on the
tailnet, Linux Mint 22.3, user `george`) — treat this directory as the source
of truth and redeploy from it.

`opti` has no public IP, so it is reached via a Cloudflare Tunnel
(`cloudflared`) rather than a direct nginx+certbot listener. See
[Tunnel and DNS](#tunnel-and-dns) below.

> **Previous home:** this receiver ran on `whoop-vm` (Oracle Cloud, public IP)
> until 2026-08-09. That VM is deprecated and currently offline. Historical
> log data for `2026-08-09` and earlier still sits on `whoop-vm` at
> `/home/george/murmur-logs/` and has **not** been migrated — that's a pending
> manual `rsync` once the VM is reachable again. See
> [Pending: historical data migration](#pending-historical-data-migration).

## Components

| File | Deploys to | Purpose |
|---|---|---|
| `murmur-logs-receiver.py` | `/home/george/murmur-logs-receiver.py` | stdlib HTTP server on `127.0.0.1:8600`: ingest, state, dashboard |
| `murmur-logs.service` | `/etc/systemd/system/` | runs the receiver as `george`, restart-always |
| `murmur-capture-watch.py` | `/home/george/murmur-capture-watch.py` | bounded stdlib aggregation of shipped capture-startup metrics |
| `dictation_lifecycle.py` | `/home/george/dictation_lifecycle.py` | deterministic per-session dictation funnel and terminal correlator |
| `murmur-capture-watch.service` | `/etc/systemd/system/` | one-shot capture regression analysis |
| `murmur-capture-watch.timer` | `/etc/systemd/system/` | runs the watch hourly with a randomized delay |
| `nginx-murmur-ingest-opti.conf` | `/etc/nginx/sites-available/murmur-ingest` (+ symlink in sites-enabled; default site removed) | local-only proxy on `127.0.0.1:8601` for `/murmur/ingest`, `/murmur/state`, `/murmur/healthz` |
| `cloudflared-config.yml` | `/etc/cloudflared/config.yml` | tunnel `opti-murmur`: routes `murmur.georgenijo.com` → `:8600` (dashboard) and `georgenijo.com` → `:8601` (nginx path rewrites) |

## Tunnel and DNS

`opti` has no public IP, so nothing listens on a public port directly.
Instead:

- **cloudflared** is installed from the official `.deb` and runs as a
  systemd service via `cloudflared service install` (not one of the units
  above — that command generates its own unit from the installed binary).
- Tunnel name: **`opti-murmur`**. Its config is `cloudflared-config.yml` in
  this directory, deployed to `/etc/cloudflared/config.yml`:
  - `murmur.georgenijo.com` → `http://127.0.0.1:8600` (dashboard, direct to
    the receiver — still gated by Cloudflare Access, see below)
  - `georgenijo.com` → `http://127.0.0.1:8601` (local nginx, which does the
    `/murmur/*` path rewrites onto the receiver)
  - catch-all → `http_status:404`
- **DNS**: both `murmur.georgenijo.com` and `georgenijo.com` are proxied
  CNAMEs to `dac9359e-51bd-4ad9-8389-dd510127c04e.cfargotunnel.com`, created with
  `cloudflared tunnel route dns --overwrite-dns opti-murmur <hostname>`.
- **Caveat:** `georgenijo.com`'s apex now routes entirely to `opti`, and
  `opti` only serves `/murmur/*` (everything else 404s from the nginx
  `location / { return 404; }` block). Anything else the old VM served on
  the bare apex is gone until it is separately migrated onto `opti` (or
  elsewhere) and added to the ingress/nginx config.
- **Cloudflare Access** app "Murmur Fleet Logs" still gates host
  `murmur.georgenijo.com`, allow policy for george.nijo8@gmail.com. (CF API
  token for Access/DNS management: macbook keychain
  `cloudflare-api-token-whoop` — name unchanged even though the VM it
  originally targeted is gone.)
- **TLS** is handled entirely by the tunnel (Cloudflare terminates it) — no
  certbot, no local certs, nothing to renew on `opti`.
- **Ingest token**: baked into the receiver and `app/src-tauri/src/log_shipper.rs`,
  unchanged by this move; reference copy in fleet secrets:
  `fleet secret get murmur-log-ingest-token`.

## Redeploy after editing the receiver

```bash
cat infra/log-receiver/murmur-logs-receiver.py | tailscale ssh george@opti "cat > /home/george/murmur-logs-receiver.py"
cat infra/log-receiver/dictation_lifecycle.py | tailscale ssh george@opti "cat > /home/george/dictation_lifecycle.py"
cat infra/log-receiver/murmur-capture-watch.py | tailscale ssh george@opti "cat > /home/george/murmur-capture-watch.py"
cat infra/log-receiver/murmur-capture-watch.service | tailscale ssh george@opti "cat > /tmp/murmur-capture-watch.service"
cat infra/log-receiver/murmur-capture-watch.timer | tailscale ssh george@opti "cat > /tmp/murmur-capture-watch.timer"
tailscale ssh george@opti "sudo install -m 0644 /tmp/murmur-capture-watch.service /etc/systemd/system/ && sudo install -m 0644 /tmp/murmur-capture-watch.timer /etc/systemd/system/ && sudo systemctl daemon-reload && sudo systemctl enable --now murmur-capture-watch.timer"
tailscale ssh george@opti "sudo systemctl restart murmur-logs && systemctl is-active murmur-logs"
tailscale ssh george@opti "sudo systemctl start murmur-capture-watch.service || true; systemctl status --no-pager murmur-capture-watch.service; systemctl list-timers --no-pager murmur-capture-watch.timer"
curl -s https://georgenijo.com/murmur/healthz   # expect: ok
```

george has passwordless sudo on `opti`, so the commands above run
non-interactively. Equivalently, any of these can run via
`fleet exec opti "..."` instead of `tailscale ssh george@opti "..."`.

Redeploying the nginx or cloudflared config after editing it in this
directory:

```bash
cat infra/log-receiver/nginx-murmur-ingest-opti.conf | tailscale ssh george@opti "sudo tee /etc/nginx/sites-available/murmur-ingest >/dev/null && sudo nginx -t && sudo systemctl reload nginx"
cat infra/log-receiver/cloudflared-config.yml | tailscale ssh george@opti "sudo tee /etc/cloudflared/config.yml >/dev/null && sudo systemctl restart cloudflared"
```

The one-shot exits `2` after atomically writing its report when it finds an
alert. That intentionally leaves an operator-visible failed unit and journal
entry; the timer continues to run on schedule. The fleet dashboard reads the
same report and shows the bounded alert details. A healthy later run clears the
failed state and replaces the report.

Rollback stops/disables the timer, removes its two units and script, reloads
systemd, and redeploys the preceding receiver. Retained `events.jsonl` remains
valid: `ingest_app_version` is an additive receiver annotation.

## Pending: historical data migration

`whoop-vm` is currently offline (decommissioned Oracle Cloud VM). Its
`/home/george/murmur-logs/` still holds all fleet log history up to
2026-08-09. Once that VM is reachable again (or its disk is otherwise
recoverable), migrate it onto `opti` with something like:

```bash
tailscale ssh george@whoop-vm "tar -C /home/george -czf - murmur-logs" | tailscale ssh george@opti "tar -C /home/george -xzf -"
```

then reconcile any per-install directories that exist on both sides (`opti`
started fresh, so most installs will simply be new directories; only
overlapping install UUIDs need care) before decommissioning `whoop-vm` for
good. This is a manual, one-time step — not automated by anything here.

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
version/backend/setup-step/outcome labels, counts, and startup durations.

## Capture regression watch

The hourly watch scans production JSONL line-by-line and keeps at most the
newest 500 readiness samples per install/version cohort and 64 explicit version
cohorts per install. Further versions collapse into a non-comparable
`overflow` cohort. It reports:

- dictation `startup_ms` p50 and p95;
- active-budget timeouts split by stable backend and native setup step;
- fallback and both-backends-failed counts;
- exhausted diagnostics-store operations split by stable operation and safe
  SQLite error class for each app-version cohort;
- a histogram of ready recordings for the five most recent completed,
  attempted dictation sessions in each cohort;
- a stable-code dictation funnel with terminal outcome counts, per-stage
  drop-off, and missing/duplicate terminal counts.

`startup_baseline` begins an app session. A session contributes to the
zero-ready signal only after a later baseline proves it ended and only when it
contained a dictation `audio initialization accepted`; idle launches, transform
captures, and the currently open session are excluded. Per-session ready counts
at or above 20 share one capped tail bucket, bounding both memory and report
cardinality.

`performance.store_operation_failed` contains only the operation, bounded
error class, attempt count, and recording ID. The watch independently
allowlists the operation and class, collapsing unknown labels before they can
enter its report. The newest affected app-version cohort produces a dashboard
alert; the versioned cohort rows retain the counts needed to distinguish a
one-off skipped run from recurrence.

An alert is created when the newest comparable version has at least five
dictation readiness samples and its p50 is more than twice the best retained
earlier eligible cohort on the same install. This baseline remains anchored
across equally slow later releases. A zero-ready alert is created only for the
latest version cohort with a completed attempted dictation session when at
least two of its five most recent attempts had no ready recording; a newer
healthy cohort supersedes stale failures and later healthy attempts age them
out. Thresholds live in the watch script and are fixture-tested. The allowlist
preserves `last_setup_step: "none"` as the explicit pre-native-call state.

## Tests

The receiver stays stdlib-only. Its plain-English classification, recovery
correlation, grouping, unknown-event fallback, exact newest-N downloads,
LLM-ready Markdown rendering, bounded retained-log activity metrics, route
bounds, HTML escaping, and dictation lifecycle correlation are covered by:

```bash
python3 -m unittest tests/test_log_receiver.py
python3 -m unittest tests/test_capture_regression_watch.py
python3 -m unittest tests/test_dictation_lifecycle.py
```

Install pages expose raw JSONL downloads for the latest 200, latest 500, or the
complete stream. The LLM-ready latest-200/latest-500 downloads use the versioned
`murmur-fleet-llm/v1` Markdown format documented in
[`docs/features/log-shipping.md`](../../docs/features/log-shipping.md).
Quick info also reports the latest proven microphone activation and latest
non-empty live transcription across the complete retained stream.
