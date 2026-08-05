# Log receiver deployment (whoop-vm)

Server side of [docs/features/log-shipping.md](../../docs/features/log-shipping.md).
Everything here is a snapshot of what runs on whoop-vm — treat this directory as
the source of truth and redeploy from it.

## Components

| File | Deploys to | Purpose |
|---|---|---|
| `murmur-logs-receiver.py` | `/home/george/murmur-logs-receiver.py` | stdlib HTTP server on `127.0.0.1:8600`: ingest, state, dashboard |
| `murmur-logs.service` | `/etc/systemd/system/` | runs the receiver as `george`, restart-always |
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
tailscale ssh ubuntu@whoop-vm "sudo systemctl restart murmur-logs && systemctl is-active murmur-logs"
curl -s https://georgenijo.com/murmur/healthz   # expect: ok
```

## Data layout

`/home/george/murmur-logs/<install-uuid>/` — `events[.dev].jsonl` (append-only),
`meta.json` (device identity), `state.json` (latest mic/device snapshot).
Caps: 8 MB/request, 200 MB/install, 10 GB global, 150 installs.
Dev-tagged batches are acked and discarded.

## Tests

The receiver stays stdlib-only. Its plain-English classification, recovery
correlation, grouping, unknown-event fallback, exact newest-N downloads,
LLM-ready Markdown rendering, route bounds, and HTML escaping are covered by:

```bash
python3 -m unittest tests/test_log_receiver.py
```

Install pages expose raw JSONL downloads for the latest 200, latest 500, or the
complete stream. The LLM-ready latest-200/latest-500 downloads use the versioned
`murmur-fleet-llm/v1` Markdown format documented in
[`docs/features/log-shipping.md`](../../docs/features/log-shipping.md).
