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
| `murmur-logs-receiver.py` | `/home/george/murmur-logs-receiver.py` | stdlib HTTP server on `127.0.0.1:8600`: ingest, state, bundle, explicit private capture, dashboard |
| `event_store.py` | `/home/george/event_store.py` | stdlib-only SQLite migrations, transactional query projection, raw segment sealing, ingest key sanitization, backfill/reconciliation, integrity, backup, and restore commands |
| `scrub_key_identities.py` | `/home/george/scrub_key_identities.py` | explicit one-shot scrub of typed-key identities from stored raw and SQLite history (dry-run by default) |
| `murmur-logs.service` | `/etc/systemd/system/` | runs the receiver as `george`, restart-always |
| `murmur-capture-watch.py` | `/home/george/murmur-capture-watch.py` | bounded stdlib aggregation of shipped capture-startup metrics; imports `event_store.py`, `dictation_lifecycle.py`, and `reliability_slo.py` from its own directory |
| `dictation_lifecycle.py` | `/home/george/dictation_lifecycle.py` | deterministic per-session dictation funnel and terminal correlator |
| `reliability_slo.py` | `/home/george/reliability_slo.py` | aggregate-only complete-week evaluator for the versioned dictation SLO contract |
| `murmur-capture-watch.service` | `/etc/systemd/system/` | one-shot capture regression analysis |
| `murmur-capture-watch.timer` | `/etc/systemd/system/` | runs the watch hourly with a randomized delay |
| `nginx-murmur-ingest-opti.conf` | `/etc/nginx/sites-available/murmur-ingest` (+ symlink in sites-enabled; default site removed) | local-only proxy on `127.0.0.1:8601` for `/murmur/ingest`, `/murmur/state`, `/murmur/bundle`, `/murmur/private-capture`, `/murmur/healthz` |
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

# Stop ingest first. The key scrub below must run before any old-client retry
# is stored, and the receiver stays stopped until it succeeds.
fleet exec opti 'sudo systemctl stop murmur-logs'

# Back up source, raw recovery archives, metadata, and any existing database.
# These backups hold unscrubbed key identities; delete them once the scrub is
# verified unless they are still needed for rollback.
fleet exec opti 'set -eu; stamp=$(date -u +%Y%m%dT%H%M%SZ); backup=/home/george/murmur-logs-backups/$stamp; mkdir -p "$backup"; cp -a /home/george/murmur-logs-receiver.py "$backup"/; test ! -e /home/george/event_store.py || cp -a /home/george/event_store.py "$backup"/; tar -C /home/george -czf "$backup/murmur-logs-raw.tgz" --exclude="murmur-logs/events.sqlite3*" murmur-logs; if test -e /home/george/murmur-logs/events.sqlite3; then python3 /home/george/event_store.py --root /home/george/murmur-logs backup "$backup/events.sqlite3"; fi; echo "$backup"'

# Stage, compile, and install every receiver-side Python file together. The
# watch imports event_store.py, dictation_lifecycle.py, and reliability_slo.py
# from its own directory, so a partial install breaks the hourly timer.
FILES="murmur-logs-receiver.py event_store.py murmur-capture-watch.py dictation_lifecycle.py reliability_slo.py scrub_key_identities.py"
LOCAL_STAGE=$(mktemp -d)
git archive "$MERGED_SHA" $(for f in $FILES; do echo infra/log-receiver/$f; done) | tar -x -C "$LOCAL_STAGE"
for f in $FILES; do fleet cp "$LOCAL_STAGE/infra/log-receiver/$f" "opti:/tmp/$f.$MERGED_SHA"; done
fleet exec opti "set -eu; cd /tmp; for f in $FILES; do python3 -m py_compile \$f.$MERGED_SHA; done; for f in $FILES; do install -m 0755 \$f.$MERGED_SHA /home/george/\$f; done"
# Review the content-free dry run, then scrub and start only if it succeeds.
fleet exec --timeout 3600 opti 'python3 /home/george/scrub_key_identities.py --root /home/george/murmur-logs'
fleet exec --timeout 3600 opti 'python3 /home/george/scrub_key_identities.py --root /home/george/murmur-logs --apply && sudo systemctl start murmur-logs && systemctl is-active murmur-logs'
curl -fsS https://georgenijo.com/murmur/healthz
```

On restart the receiver first seals any live `events.jsonl` already at the
200 MiB cap (an instant rename), then compresses pending segments in the
background. Check the journal for `raw compaction failed` lines.

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
fleet exec opti 'result_code=0; report=/home/george/murmur-logs/reconciliation.json; temporary=$report.tmp; sudo systemctl stop murmur-logs || exit $?; python3 /home/george/event_store.py --root /home/george/murmur-logs backfill --max-lines 100000 || result_code=$?; if test "$result_code" -eq 0; then python3 /home/george/event_store.py --root /home/george/murmur-logs reconcile --mark-ready >"$temporary" || result_code=$?; fi; if test "$result_code" -eq 0; then mv "$temporary" "$report"; else rm -f "$temporary"; fi; sudo systemctl start murmur-logs || exit $?; test "$result_code" -eq 0 || exit "$result_code"; systemctl is-active murmur-logs'
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
directory. Compare first: as of 2026-10-27 the live
`/etc/nginx/sites-available/murmur-ingest` also serves the static
`/var/www/georgenijo.com` site and `www.georgenijo.com`, and has no
`/murmur/private-capture` route. Overwriting it with this file removes the
static site; merge the two deliberately rather than piping blindly:

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
  raw history. Limits are admission controls: 8 MiB/request, a 200 MiB live
  `events.jsonl` per install (hang-diagnostics bundles are admitted against
  the same figure), 10 GiB total raw install directories (every file,
  including compressed sealed segments), 150 installs, and 10 GiB for SQLite.
  Raising a limit is an explicit reviewed operator change.
- A batch that would push the live file past 200 MiB seals it instead of
  returning `507`. See [Raw segment sealing](#raw-segment-sealing).
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
  publishes it atomically. Back up JSONL, `sealed/` segments, and metadata
  with it because the raw stream is still the recovery/export source.
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

`/home/george/murmur-logs/<install-uuid>/` contains the production live
`events.jsonl` (append-only), immutable sealed segments under `sealed/`,
bounded `meta.json`, and `state.json` with only the current privacy-safe
aggregate microphone contract. An install's raw stream is its sealed segments
in sequence order followed by the live file. The older `archive/` directory
some installs have holds hang bundles only and is unrelated. The root
contains `events.sqlite3`, its transient WAL/SHM files, the content-free
`reconciliation.json`, and `.raw-maintenance.lock`. Dev-tagged batches are
acknowledged and discarded; historical `events.dev.jsonl` files are not
backfilled.

### Raw segment sealing

When an accepted batch would push `events.jsonl` past 200 MiB, ingest renames
it to `sealed/events-NNNNNN.pending.jsonl` inside the same SQLite write
transaction that serializes every append, then writes the batch to a fresh
live file. The rename is the commit point, so a crash leaves either the old
live file or a pending segment, never a partial copy. A failed commit after a
seal removes only the new batch's lines; the seal stays because it moves bytes
without changing the stream.

A background pass then streams the pending file into a fsynced gzip
temporary, decompresses it again to verify the size and SHA-256, atomically
publishes `sealed/events-NNNNNN-<first>-<last>.jsonl.gz`, and only then
removes the pending file. `<first>`/`<last>` are the UTC timestamps of the
segment's first and last lines (`untimed` if absent). Every intermediate state
is a valid stream: temporaries are never read, a published gzip wins over a
same-sequence pending file, and the next pass verifies and removes that
duplicate. A gap in the sequence or a gzip that differs from its pending file
fails closed. Startup seals any install already at the cap before serving and
resumes interrupted compaction.

Exact-retry deduplication is unaffected because it is enforced by SQLite's
`UNIQUE(install_id, event_hash)`, not by the raw file. Backfill checkpoint keys
follow the byte stream: sequence 1 keeps the historical
`<uuid>/events.jsonl` key and later sequences use `<uuid>/events.jsonl#N`, so a
live file keeps its key when it is sealed and no checkpoint is rewritten.
Backfill and reconciliation read every segment, and reconciliation compares
one hash set per install across all of them. The complete JSONL download
streams the decompressed segments followed by the live file, with handles and
lengths fixed before headers are sent. Latest-200/500, LLM-ready reports, the
activity metrics, and the hourly watch also span segments.

### Typed-key identity sanitization and scrub

App versions up to 0.52.2 shipped keyboard telemetry carrying rdev Debug
forms such as `KeyPress(KeyI)`, `KeyRelease(Num4)`, `Some(Slash)`, or a bare
`SemiColon` in key fields, and `Some("a")` key names. The shared
`event_store.prepare_event` normalization replaces these with
`KeyPress(non_hotkey_key)`, `Some(non_hotkey_key)`, `non_hotkey_key`, or
`Some(redacted_text)` before hashing, so ingest writes only the sanitized form
to both JSONL and SQLite and a retry from an old client deduplicates against
it. Configured hotkeys (`ShiftLeft`/`ShiftRight`, `ControlLeft`/`ControlRight`,
`Alt`, `AltGr`, `MetaLeft`/`MetaRight`, `Function`, F1-F20) and unrelated
values such as `Some(Idle)` detector states are kept.

History stored before that change needs the one-shot scrub tool:

```bash
# Dry run (the default): content-free per-install counts, changes nothing.
fleet exec --timeout 3600 opti 'python3 /home/george/scrub_key_identities.py --root /home/george/murmur-logs'

# Apply with the receiver stopped (the tool refuses while it holds its lock).
# `&&` keeps the receiver stopped if the scrub fails; rerun the scrub, never
# start the receiver around a failure.
fleet exec --timeout 3600 opti 'sudo systemctl stop murmur-logs && python3 /home/george/scrub_key_identities.py --root /home/george/murmur-logs --apply && sudo systemctl start murmur-logs'
```

Sanitization matches rdev forms embedded anywhere in a string, including
summaries, on token boundaries (`MyKeyPress(KeyA)`, `/models/KeyA/`, or
`KeyA-7B` are untouched). It also redacts character-bearing name fields:
`event_name` everywhere, and `name`/`char`/`text`-style fields of
`stream: "keyboard"` events.

`--apply` rewrites the live file, every sealed gzip segment, and any legacy
`events.dev.jsonl` through a fsynced temporary and an atomic rename. Raw lines
that become identical to another line once sanitized keep only their first
occurrence in the install's stream. It then
updates matching SQLite rows (canonical JSON, event hash, and summary/FTS) in
one rowid-ordered pass and a single transaction with `secure_delete` on,
optimizes FTS when a summary changed, and truncates the WAL. Rows that
differed only by the typed key become identical and are merged into the
earliest stored row. A small `.key-scrub-journal.json` records each file
rewrite so an interrupted run finishes the matching backfill-checkpoint update
when rerun. While that journal exists the receiver refuses to start, so no
ingest can change a half-scrubbed file. Reconciliation continues to prove the
projection afterwards.

The tool deliberately keeps no unscrubbed copy, because a retained backup
would retain the key identities being removed. It does not touch existing
operator backups (for example `/home/george/murmur-logs-backups/`), which still
hold the unscrubbed data until someone deletes them.

### Server-armed hang diagnostics control files

Dormant by default (see
[docs/features/log-shipping.md](../../docs/features/log-shipping.md)). Two
plain-text files live directly under `/home/george/murmur-logs/` (not inside
any install directory) and are re-read fresh on every `/ingest` and `/bundle`
request — never cached, so an edit takes effect on the armed install's very
next request, no restart needed:

- `diag-installs.txt` — one install UUID per line. An install listed here
  gets `{"diagnostics": true, ...}` in its `/ingest` success reply instead of
  a bare `204`, and may then `POST /bundle`. Remove its line to disarm.
- `collect-now.txt` — lines of `<uuid> <epoch>`. An epoch greater than any
  epoch that install has already honored triggers exactly one immediate
  probe-bundle collection on that armed install; malformed lines are
  ignored. Clear an install's line once its bundle arrives to avoid a
  repeat collection on the app's next launch.

An armed install's uploaded bundles land at
`/home/george/murmur-logs/<install-uuid>/hang-bundle-<epoch-ms>.txt` — an
opaque text file written verbatim from the request body. It counts against
that install's existing 200 MiB raw-storage cap, and it never enters SQLite,
the dashboard, or search.

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
- successful stop-to-delivery-attempt p50 and p95, plus a 1 to 15 second
  target cohort with a 20-sample floor, p50 below 1,000 ms, and p95 below
  2,000 ms;
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
`slo_contract: 1` can enter the `murmur-reliability-slo/v2` report;
pre-contract data is always insufficient. A complete week needs 200 eligible
requests and 99.5% at or below 400 ms. Permission prompts and non-prompt user
cancellations while still starting before the 400 ms deadline are reported as
separate startup exclusions. Cancellations at or after the deadline remain
misses. Restart-boundary state classification and actionable-presentation
coverage still include every request. The newest two complete sufficient
passing weeks are the only path to the two-week finish-line flag.

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

The watch also alerts when the newest stop-to-delivery target cohort on an
install has at least 20 eligible samples and misses either strict percentile
target. Eligible samples are successful, non-empty native dictations with 1 to
15 seconds of audio. The metric ends after Murmur's configured clipboard or
paste attempt. It does not prove that another application consumed a posted
paste event.

## Tests

The receiver stays stdlib-only. Its plain-English classification, recovery
correlation, grouping, unknown-event fallback, exact newest-N downloads,
LLM-ready Markdown rendering, bounded retained-log activity metrics, route
bounds, HTML escaping, and dictation lifecycle correlation are covered by:

```bash
python3 -m unittest tests/test_event_store.py
python3 -m unittest tests/test_log_receiver.py
python3 -m unittest tests/test_receiver_hang_diagnostics.py
python3 -m unittest tests/test_receiver_key_scrub.py
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
