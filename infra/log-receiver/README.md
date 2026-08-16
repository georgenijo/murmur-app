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
| `event_store.py` | `/home/george/event_store.py` | stdlib-only SQLite migrations, transactional query projection, backfill/reconciliation, integrity, backup, and restore commands |
| `murmur-logs.service` | `/etc/systemd/system/` | runs the receiver as `george`, restart-always |
| `murmur-capture-watch.py` | `/home/george/murmur-capture-watch.py` | bounded stdlib aggregation of shipped capture-startup metrics |
| `dictation_lifecycle.py` | `/home/george/dictation_lifecycle.py` | deterministic per-session dictation funnel and terminal correlator |
| `reliability_slo.py` | `/home/george/reliability_slo.py` | aggregate-only complete-week evaluator for the versioned dictation SLO contract |
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

## SQLite rollout and exact-source deployment

The production query store is `/home/george/murmur-logs/events.sqlite3`. Each
HTTP request gets its own configured connection. The store uses foreign keys,
WAL, `synchronous=FULL`, a two-second busy timeout, a 2 MiB connection cache,
a 10 GiB database quota, and a 16 MiB WAL journal limit. FTS5 indexes only the
already-sanitized event summary; the full sanitized object remains in
`events.event_json`.

Deploy from the exact merged commit, not a moving checkout or a patch against
the older deployed receiver. Replace `<merge-sha>` with GitHub's recorded merge
commit:

```bash
MERGED_SHA=<merge-sha>
git fetch origin
git cat-file -e "$MERGED_SHA^{commit}"

# Back up source, raw recovery archives, metadata, and any existing database.
fleet exec opti 'set -eu; stamp=$(date -u +%Y%m%dT%H%M%SZ); backup=/home/george/murmur-logs-backups/$stamp; mkdir -p "$backup"; cp -a /home/george/murmur-logs-receiver.py "$backup"/; test ! -e /home/george/event_store.py || cp -a /home/george/event_store.py "$backup"/; tar -C /home/george -czf "$backup/murmur-logs-raw.tgz" --exclude="murmur-logs/events.sqlite3*" murmur-logs; if test -e /home/george/murmur-logs/events.sqlite3; then python3 /home/george/event_store.py --root /home/george/murmur-logs backup "$backup/events.sqlite3"; fi; echo "$backup"'

# Stage, compile, and install both exact source files together.
LOCAL_STAGE=$(mktemp -d)
git archive "$MERGED_SHA" infra/log-receiver/murmur-logs-receiver.py infra/log-receiver/event_store.py | tar -x -C "$LOCAL_STAGE"
fleet cp "$LOCAL_STAGE/infra/log-receiver/event_store.py" opti:/tmp/event_store.py.$MERGED_SHA
fleet cp "$LOCAL_STAGE/infra/log-receiver/murmur-logs-receiver.py" opti:/tmp/murmur-logs-receiver.py.$MERGED_SHA
fleet exec opti "set -eu; python3 -m py_compile /tmp/event_store.py.$MERGED_SHA /tmp/murmur-logs-receiver.py.$MERGED_SHA; install -m 0755 /tmp/event_store.py.$MERGED_SHA /home/george/event_store.py; install -m 0755 /tmp/murmur-logs-receiver.py.$MERGED_SHA /home/george/murmur-logs-receiver.py"
fleet exec opti 'sudo systemctl restart murmur-logs && systemctl is-active murmur-logs'
curl -fsS https://georgenijo.com/murmur/healthz
```

The first restart migrates SQLite but leaves `dashboard_ready=0`. New
production ingests transactionally update SQLite and raw JSONL while dashboard
reads deliberately stay on JSONL. Run the bounded, checkpointed backfill until
it reports `"complete":true`:

```bash
fleet exec opti 'python3 /home/george/event_store.py --root /home/george/murmur-logs backfill --max-lines 100000'
```

Stop the receiver briefly for the final proof so the source snapshot cannot
grow. A failed reconciliation leaves the readiness flag unchanged:

```bash
fleet exec opti 'status=0; report=/home/george/murmur-logs/reconciliation.json; temporary=$report.tmp; sudo systemctl stop murmur-logs || exit $?; python3 /home/george/event_store.py --root /home/george/murmur-logs backfill --max-lines 100000 || status=$?; if test "$status" -eq 0; then python3 /home/george/event_store.py --root /home/george/murmur-logs reconcile --mark-ready >"$temporary" || status=$?; fi; if test "$status" -eq 0; then mv "$temporary" "$report"; else rm -f "$temporary"; fi; sudo systemctl start murmur-logs || exit $?; test "$status" -eq 0 || exit "$status"; systemctl is-active murmur-logs'
```

The versioned reconciliation report is content-free. Per install it records
raw lines, valid objects, malformed/non-object lines, duplicate hashes,
untimed events, backfill insertions, database rows, and earliest/latest valid
timestamps. Unique valid-object counts, untimed counts, timestamp bounds, and
the complete event-hash set must match SQLite for every available production
archive before indexed reads can be enabled.

Verify the unchanged exposure boundary. An unauthenticated dashboard request
must enter Cloudflare Access, while nginx's public apex still returns 404 for
a dashboard path:

```bash
curl -sS -o /dev/null -w '%{http_code}\n' https://murmur.georgenijo.com/
curl -sS -o /dev/null -w '%{http_code}\n' https://georgenijo.com/murmur/search
```

Use an Access-authenticated browser to exercise an arbitrary date search and a
second keyset page. Never print or publish production event rows while
validating.

### Isolated staging on `opti`

Stage only synthetic data under a temporary root on an unused loopback port.
The overrides do not change production service defaults:

```bash
fleet exec opti 'root=$(mktemp -d /tmp/murmur-434-stage.XXXXXX); MURMUR_LOG_ROOT="$root" MURMUR_LOG_PORT=18600 python3 /home/george/murmur-logs-receiver.py'
```

Use a named tmux session for a longer staging run, check the port first, and
remove the temporary root only after recording the content-free results.

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

## Retention, failure handling, backup, restore, and rollback

- History is retained indefinitely. Nothing automatically deletes database or
  raw history. Limits are admission controls: 8 MiB/request, 200 MiB raw JSONL
  per install, 10 GiB total raw install directories, 150 installs, and 10 GiB
  for SQLite. Raising a limit is an explicit reviewed operator change.
- The receiver parses a complete batch before mutation. New event hashes enter
  one `BEGIN IMMEDIATE` transaction, their raw lines are appended and fsynced,
  and SQLite commits last. An exact retry inserts and appends nothing. Archive
  or commit failure rolls back SQLite and truncates JSONL to its original byte
  boundary. Busy, quota, `SQLITE_FULL`, corruption, and commit errors return
  non-2xx, leaving the shipper offset unchanged.
- Disk full is not repaired by deleting history. Reject ingest, expand storage
  or remove unrelated data, run integrity checking, and let clients retry. Do
  not use a 2xx override.
- Create an online database backup with
  `event_store.py --root /home/george/murmur-logs backup <destination>`. The
  command uses SQLite's backup API, integrity-checks and fsyncs the result, and
  publishes it atomically. Back up JSONL and metadata with it because JSONL is
  still the recovery/export source.
- Run a full check with
  `event_store.py --root /home/george/murmur-logs integrity`. Startup also runs
  `quick_check`; corruption prevents receiver startup and ingest
  acknowledgement.
- Restore only while `murmur-logs.service` is stopped. Preserve the suspect
  database plus WAL/SHM for investigation, then run
  `event_store.py --root /home/george/murmur-logs restore <verified-backup>`.
  Restore validates schema and integrity, creates a SQLite-consistent
  `.pre-restore-<UTC>` copy when possible (or an exact forensic copy when the
  current database is corrupt), preserves any WAL/SHM beside that copy,
  atomically installs the backup, and removes stale live WAL/SHM files. Run
  `integrity`, restart, and reconcile before enabling indexed reads.
- Query rollback is immediate and non-destructive:
  `event_store.py --root /home/george/murmur-logs disable-dashboard` switches
  dashboard reads back to JSONL. For code rollback, install the two source
  files saved before deployment (or the exact prior commit) and restart. Raw
  JSONL remains compatible; retain SQLite for diagnosis. Restoring an older
  database is not required for a code rollback.

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

`/home/george/murmur-logs/<install-uuid>/` contains production
`events.jsonl` (append-only), bounded `meta.json`, and `state.json` with only
the current privacy-safe aggregate microphone contract. The root contains
`events.sqlite3`, its transient WAL/SHM files, and the content-free
`reconciliation.json`. Dev-tagged batches are acknowledged and discarded;
historical `events.dev.jsonl` files are not backfilled.

Accepted production events are annotated with the bounded
`ingest_app_version` already supplied in the request header. This adds no
client-collected field. Pre-deployment retained lines remain unmodified and the
watch groups them under `unknown`, which is never used for version comparisons.

`capture-watch.json` is a versioned, atomically replaced derived report. Its
legacy cohort/alert rows contain no source event text or device metadata: only
install UUID, bounded app version/backend/setup-step/outcome labels, counts,
and startup durations. Its `reliability_slo` subtree is stricter and
aggregate-only: install UUIDs and app versions are internal joins and never
enter that subtree or its dashboard card.

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

The same full scan feeds `ReliabilitySloEvaluator` in source order. Production
emits the accepted request immediately before `idle` → `starting`, keeping the
transition telemetry cost inside request-to-first-PCM latency. The evaluator
still tolerates late or reordered transport and joins internally by install,
`startup_baseline` app session, and positive recording ID, then publishes only
weekly aggregate counts. It recomputes one partial and eight complete UTC weeks
every run so late arrivals update their original window. Only requests marked
`slo_contract: 1` count; pre-contract data is always insufficient. A complete
week needs 200 eligible requests and 99.5% at or below 400 ms, has explicit
permission-prompt exclusion, restart-boundary state classification, and
actionable-presentation coverage. The newest two complete sufficient passing
weeks are the only path to the two-week finish-line flag.

Contract requests with invalid or future timestamps are not assigned to guessed
windows. Valid history older than the retained horizon simply expires. Invalid
requests increment a bounded aggregate integrity count, force the two-week flag
false, and make the one-shot watch alert. Evidence beyond fixed evaluator
memory/cardinality bounds increments a separate aggregate overflow count and
fails closed the same way. Malformed JSON and non-object lines in the retained
`events.jsonl` input increment a bounded malformed-source count and fail closed;
the next complete full scan clears a transient partial-tail error. The dashboard
validates the exact report shape and cross-field arithmetic before rendering;
malformed or contradictory derived reports are shown as unavailable rather
than trusted.

Correlated non-request lifecycle evidence with a malformed or future timestamp
is ignored as proof and counted in its request week's
`invalid_evidence_timestamps`. That week is indeterminate rather than allowing
future accepted/ready/prompt/terminal/presentation/state records to manufacture
a healthy result.

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
python3 -m unittest tests/test_event_store.py
python3 -m unittest tests/test_log_receiver.py
python3 -m unittest tests/test_capture_regression_watch.py
python3 -m unittest tests/test_dictation_lifecycle.py
python3 -m unittest tests/test_reliability_slo.py
```

Install pages expose raw JSONL downloads for the latest 200, latest 500, or the
complete stream. The LLM-ready latest-200/latest-500 downloads use the versioned
`murmur-fleet-llm/v1` Markdown format documented in
[`docs/features/log-shipping.md`](../../docs/features/log-shipping.md).
Quick info also reports the latest proven microphone activation and latest
non-empty live transcription across the complete retained stream.
