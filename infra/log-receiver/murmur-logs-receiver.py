#!/usr/bin/env python3
"""Murmur log ingest receiver + fleet dashboard.

Accepts NDJSON batches from murmur-app installs and appends them to
per-install JSONL files under ~/murmur-logs/. Stdlib only.

    POST /ingest
      Authorization: Bearer <token>
      X-Install-Id: <uuid4>
      X-App-Version: <semver>   (optional)
      X-Dev: 1                  (optional, dev builds)
      body: NDJSON (one JSON object per line)

    GET /dashboard   — HTML overview of every install (gate behind CF Access)
    GET /healthz     — liveness

Responses: 204 ok, 401 bad token, 400 bad payload, 413 too large.
"""

import html
import json
import os
import re
import sys
import time
from datetime import datetime
from zoneinfo import ZoneInfo
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

TOKEN = "a1b4068693a1f3868bcf03c01ebcf1e9f000080b3e8bfcb0"
ROOT = os.path.expanduser("~/murmur-logs")
MAX_BODY = 8 * 1024 * 1024  # 8 MB
MAX_FILE = 200 * 1024 * 1024  # per-install cap: stop appending past 200 MB
INSTALL_ID_RE = re.compile(r"^[0-9a-fA-F-]{8,64}$")
MAX_INSTALLS = 150
MAX_TOTAL = 10 * 1024 * 1024 * 1024  # 10 GB across all installs
_usage_cache = {"t": 0.0, "bytes": 0, "dirs": 0}


def root_usage():
    """Total bytes and dir count under ROOT, cached for 60s."""
    if time.time() - _usage_cache["t"] > 60:
        total = dirs = 0
        for entry in os.listdir(ROOT) if os.path.isdir(ROOT) else []:
            d = os.path.join(ROOT, entry)
            if not os.path.isdir(d):
                continue
            dirs += 1
            for f in os.listdir(d):
                try:
                    total += os.path.getsize(os.path.join(d, f))
                except OSError:
                    pass
        _usage_cache.update(t=time.time(), bytes=total, dirs=dirs)
    return _usage_cache


def atomic_write_json(path, obj):
    tmp = path + ".tmp"
    try:
        with open(tmp, "w") as f:
            json.dump(obj, f)
        os.replace(tmp, path)
    except OSError:
        pass
EASTERN = ZoneInfo("America/New_York")


def eastern_time(ts):
    """'2026-07-30T13:42:15.192Z' -> '09:42:15' Eastern."""
    try:
        dt = datetime.fromisoformat(ts.replace("Z", "+00:00"))
        return dt.astimezone(EASTERN).strftime("%H:%M:%S")
    except (ValueError, TypeError):
        return str(ts)[11:19]


def tail_lines(path, n, max_bytes=64 * 1024):
    """Last n lines of a file, reading at most max_bytes from the end."""
    try:
        size = os.path.getsize(path)
        with open(path, "rb") as f:
            f.seek(max(0, size - max_bytes))
            chunk = f.read().decode("utf-8", "replace")
        lines = [l for l in chunk.split("\n") if l.strip()]
        return lines[-n:]
    except OSError:
        return []


def count_lines(path):
    try:
        with open(path, "rb") as f:
            return sum(buf.count(b"\n") for buf in iter(lambda: f.read(1 << 20), b""))
    except OSError:
        return 0


def collect_installs():
    installs = []
    if not os.path.isdir(ROOT):
        return installs
    for entry in sorted(os.listdir(ROOT)):
        d = os.path.join(ROOT, entry)
        if not os.path.isdir(d):
            continue
        meta = {}
        try:
            with open(os.path.join(d, "meta.json")) as f:
                meta = json.load(f)
        except (OSError, ValueError):
            pass
        state = {}
        try:
            with open(os.path.join(d, "state.json")) as f:
                state = json.load(f)
        except (OSError, ValueError):
            pass
        for fname in ("events.jsonl", "events.dev.jsonl"):
            path = os.path.join(d, fname)
            if not os.path.exists(path):
                continue
            last = tail_lines(path, 1)
            last_event, last_ts = {}, ""
            if last:
                try:
                    last_event = json.loads(last[-1])
                    last_ts = last_event.get("timestamp", "")
                except ValueError:
                    pass
            installs.append({
                "id": entry,
                "kind": "dev" if ".dev." in fname else "prod",
                "device": meta.get("device_name", ""),
                "os": meta.get("os", ""),
                "hw": meta.get("hw", ""),
                "specs": meta.get("specs", ""),
                "mic": state.get("default_input") or "",
                "version": meta.get("last_version", "?"),
                "events": count_lines(path),
                "bytes": os.path.getsize(path),
                "mtime": os.path.getmtime(path),
                "first_seen": os.path.getctime(d),
                "last_ts": last_ts,
                "last_summary": str(last_event.get("summary", ""))[:110],
            })
    installs.sort(key=lambda i: i["mtime"], reverse=True)
    return installs


def ago(ts):
    s = int(time.time() - ts)
    if s < 90:
        return "%ds ago" % s
    if s < 5400:
        return "%dm ago" % (s // 60)
    if s < 172800:
        return "%dh ago" % (s // 3600)
    return "%dd ago" % (s // 86400)


def render_dashboard():
    installs = collect_installs()
    now = time.time()
    rows = []
    for i in installs:
        fresh = now - i["mtime"] < 300
        is_new = now - i["first_seen"] < 86400
        dot = "#4ade80" if fresh else "#64748b"
        badge = ' <span class="new">NEW</span>' if is_new else ""
        kind_cls = "dev" if i["kind"] == "dev" else "prod"
        device = html.escape(i["device"]) if i["device"] else "&mdash;"
        os_hw = " · ".join(x for x in (i["os"], i["specs"] or i["hw"]) if x)
        rows.append(
            '<tr>'
            '<td><span class="dot" style="background:%s"></span>'
            '<a href="/install/%s?kind=%s"><strong>%s</strong></a>'
            ' <span class="badge %s">%s</span>%s'
            '<div class="meta">%s &middot; %s events &middot; %.1f&thinsp;MB%s</div></td>'
            '<td class="num">v%s</td>'
            '<td>%s</td>'
            '</tr>'
            % (
                dot,
                html.escape(i["id"]),
                i["kind"],
                device,
                kind_cls,
                i["kind"],
                badge,
                html.escape(os_hw) or "&mdash;",
                "{:,}".format(i["events"]),
                i["bytes"] / 1048576,
                " &middot; &#127908; " + html.escape(i["mic"]) if i["mic"] else "",
                html.escape(str(i["version"])),
                ago(i["mtime"]),
            )
        )
    body = (
        "<h1>murmur fleet logs</h1>"
        "<p class='sub'>%d install stream%s · refreshes every 30s · %s</p>"
        "<table><thead><tr><th>device</th><th>version</th>"
        "<th>last event</th></tr></thead>"
        "<tbody>%s</tbody></table>"
        % (
            len(installs),
            "" if len(installs) == 1 else "s",
            datetime.now(EASTERN).strftime("%Y-%m-%d %-I:%M %p ET"),
            "".join(rows) or '<tr><td colspan="3">no installs yet</td></tr>',
        )
    )
    return """<!doctype html><html><head><meta charset="utf-8">
<meta http-equiv="refresh" content="30">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>murmur fleet logs</title><style>
body{background:#0b1120;color:#e2e8f0;font:15px/1.5 -apple-system,system-ui,sans-serif;margin:2rem auto;max-width:70rem;padding:0 1rem}
h1{font-size:1.3rem;margin:0}
.sub{color:#64748b;margin:.3rem 0 1.4rem;font-size:.85rem}
table{border-collapse:collapse;width:100%%}
th{text-align:left;color:#64748b;font-size:.75rem;text-transform:uppercase;letter-spacing:.05em;padding:.4rem .7rem;border-bottom:1px solid #1e293b}
td{padding:.55rem .7rem;border-bottom:1px solid #16213a;vertical-align:top}
td.num{text-align:right;font-variant-numeric:tabular-nums}
code{color:#93c5fd;font-size:.85em}
.summary code{color:#94a3b8}
.dot{display:inline-block;width:8px;height:8px;border-radius:50%%;margin-right:.5rem}
.badge{font-size:.7rem;padding:.1rem .45rem;border-radius:99px;text-transform:uppercase;letter-spacing:.04em}
.badge.prod{background:#14532d;color:#86efac}
.badge.dev{background:#3b2f14;color:#fbbf24}
.new{font-size:.65rem;color:#0b1120;background:#4ade80;border-radius:99px;padding:.1rem .4rem;margin-left:.5rem;font-weight:700}
.meta{color:#64748b;font-size:.75rem;margin-top:.15rem}
a{color:#e2e8f0;text-decoration:none}a:hover{text-decoration:underline}
tr.warn td{background:#2a1e0a}tr.error td{background:#2a0f14}
.lvl{font-size:.7rem;padding:.05rem .4rem;border-radius:4px}
.lvl.info{color:#94a3b8}.lvl.warn{background:#3b2f14;color:#fbbf24}.lvl.error{background:#450a0a;color:#f87171}
.stream{color:#818cf8;font-size:.8em}
.back{color:#64748b;font-size:.85rem}
</style></head><body>%s</body></html>""" % body


def render_install(install_id, kind, n=200):
    n = max(50, min(n, 5000))
    fname = "events.dev.jsonl" if kind == "dev" else "events.jsonl"
    path = os.path.join(ROOT, install_id, fname)
    if not os.path.exists(path):
        return None
    meta = {}
    try:
        with open(os.path.join(ROOT, install_id, "meta.json")) as f:
            meta = json.load(f)
    except (OSError, ValueError):
        pass
    total = count_lines(path)
    size_mb = os.path.getsize(path) / 1048576
    events = []
    for line in tail_lines(path, n, max_bytes=max(512 * 1024, n * 600)):
        try:
            events.append(json.loads(line))
        except ValueError:
            continue
    problems = [e for e in events if e.get("level") in ("warn", "error")]

    def row(e):
        lvl = e.get("level", "info")
        cls = lvl if lvl in ("warn", "error") else ""
        data = e.get("data") or {}
        data_str = " ".join("%s=%s" % (k, v) for k, v in list(data.items())[:6])
        return (
            '<tr class="%s"><td class="num">%s</td>'
            '<td><span class="stream">%s</span></td>'
            '<td><span class="lvl %s">%s</span></td>'
            '<td><code>%s</code><div class="meta">%s</div></td></tr>'
            % (
                cls,
                html.escape(eastern_time(str(e.get("timestamp", "")))),
                html.escape(str(e.get("stream", ""))),
                lvl,
                lvl,
                html.escape(str(e.get("summary", ""))[:160]),
                html.escape(data_str[:160]),
            )
        )

    title = meta.get("device_name") or install_id[:8]
    sub = " · ".join(
        x for x in (
            meta.get("os", ""),
            meta.get("specs", "") or meta.get("hw", ""),
            "v" + meta.get("last_version", "?"),
            kind,
            install_id,
        ) if x
    )
    info_html = ""
    try:
        with open(os.path.join(ROOT, install_id, "state.json")) as f:
            state = json.load(f)
        others = [d for d in state.get("input_devices", []) if d != state.get("default_input")]
        chips = []
        for ev in reversed(events):
            if str(ev.get("summary", "")).startswith("configure_dictation"):
                data = ev.get("data") or {}
                chips = ["%s: %s" % (k, v) for k, v in sorted(data.items())][:12]
                break
        info_html = (
            '<h2>quick info</h2><table><tbody>'
            '<tr><td>Microphone</td><td><strong>%s</strong></td></tr>'
            '<tr><td>Other inputs</td><td>%s</td></tr>'
            '<tr><td>Settings</td><td class="meta">%s</td></tr>'
            '<tr><td>As of</td><td>%s</td></tr>'
            '</tbody></table>'
            % (
                html.escape(state.get("default_input") or "unknown"),
                html.escape(", ".join(others)) or "&mdash;",
                html.escape(" · ".join(chips)) or "&mdash;",
                ago(state.get("received_at", 0)),
            )
        )
    except (OSError, ValueError):
        pass
    problems_html = ""
    if problems:
        problems_html = (
            "<h2>recent warnings &amp; errors (%d)</h2><table><tbody>%s</tbody></table>"
            % (len(problems), "".join(row(e) for e in reversed(problems[-25:])))
        )
    base = "/install/%s" % install_id
    body = (
        '<p class="back"><a href="/">&larr; all devices</a></p>'
        "<h1>%s</h1><p class='sub'>%s</p>"
        "<p class='sub'>%s events on server (%.1f MB) &middot; "
        "show last <a href='%s?kind=%s&n=200'>200</a> / "
        "<a href='%s?kind=%s&n=1000'>1,000</a> / "
        "<a href='%s?kind=%s&n=5000'>5,000</a> &middot; "
        "<a href='%s/raw?kind=%s'>&darr; download entire log (.jsonl)</a></p>%s%s"
        "<h2>last %d events (newest first)</h2>"
        "<table><tbody>%s</tbody></table>"
        % (
            html.escape(title),
            html.escape(sub),
            "{:,}".format(total),
            size_mb,
            base, kind, base, kind, base, kind, base, kind,
            info_html,
            problems_html,
            len(events),
            "".join(row(e) for e in reversed(events)),
        )
    )
    page = render_dashboard()  # reuse the <style> shell
    return page[: page.index("<body>") + 6] + body + "</body></html>"


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "murmur-logs"

    def log_message(self, fmt, *args):
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))

    def _reply(self, code, msg=b"", ctype="text/plain"):
        self.send_response(code)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(msg)))
        self.end_headers()
        if msg:
            self.wfile.write(msg)

    def do_POST(self):
        if self.path == "/state":
            return self._do_state()
        if self.path != "/ingest":
            return self._reply(404)
        auth = self.headers.get("Authorization", "")
        if auth != "Bearer " + TOKEN:
            return self._reply(401)
        install_id = self.headers.get("X-Install-Id", "")
        if not INSTALL_ID_RE.match(install_id):
            return self._reply(400, b"bad install id")
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            return self._reply(400, b"bad length")
        if length <= 0:
            return self._reply(400, b"empty")
        if length > MAX_BODY:
            return self._reply(413)
        body = self.rfile.read(length)

        # Dev builds are not part of the fleet; ack so old debug builds
        # advance their offset, but store nothing.
        if self.headers.get("X-Dev") == "1":
            return self._reply(204)

        lines = []
        for raw in body.split(b"\n"):
            raw = raw.strip()
            if not raw:
                continue
            try:
                json.loads(raw)
            except ValueError:
                return self._reply(400, b"bad json line")
            lines.append(raw)
        if not lines:
            return self._reply(400, b"empty")

        suffix = ".dev" if self.headers.get("X-Dev") == "1" else ""
        dirpath = os.path.join(ROOT, install_id.lower())
        usage = root_usage()
        if usage["bytes"] > MAX_TOTAL:
            return self._reply(507, b"global quota exceeded")
        if not os.path.isdir(dirpath) and usage["dirs"] >= MAX_INSTALLS:
            return self._reply(507, b"install limit reached")
        os.makedirs(dirpath, exist_ok=True)
        path = os.path.join(dirpath, "events%s.jsonl" % suffix)
        try:
            if os.path.exists(path) and os.path.getsize(path) > MAX_FILE:
                return self._reply(507, b"install quota exceeded")
        except OSError:
            pass
        meta_path = os.path.join(dirpath, "meta.json")
        meta = {}
        try:
            with open(meta_path) as f:
                meta = json.load(f)
        except (OSError, ValueError):
            pass
        updates = {
            "last_version": self.headers.get("X-App-Version", ""),
            "device_name": self.headers.get("X-Device-Name", ""),
            "os": self.headers.get("X-Os-Version", ""),
            "hw": self.headers.get("X-Hw-Model", ""),
            "specs": self.headers.get("X-Hw-Specs", ""),
        }
        for k, v in updates.items():
            if v:
                if k == "device_name":
                    v = re.sub(r"(\w)\?(s\b)", r"\1'\2", v)
                meta[k] = v[:120]
        atomic_write_json(meta_path, meta)
        with open(path, "ab") as f:
            f.write(b"\n".join(lines) + b"\n")
        self._reply(204)

    def _do_state(self):
        if self.headers.get("Authorization", "") != "Bearer " + TOKEN:
            return self._reply(401)
        install_id = self.headers.get("X-Install-Id", "")
        if not INSTALL_ID_RE.match(install_id):
            return self._reply(400, b"bad install id")
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except ValueError:
            return self._reply(400)
        if not 0 < length <= 32 * 1024:
            return self._reply(413)
        try:
            state = json.loads(self.rfile.read(length))
            assert isinstance(state, dict)
        except (ValueError, AssertionError):
            return self._reply(400, b"bad json")
        state["received_at"] = time.time()
        dirpath = os.path.join(ROOT, install_id.lower())
        usage = root_usage()
        if not os.path.isdir(dirpath) and usage["dirs"] >= MAX_INSTALLS:
            return self._reply(507, b"install limit reached")
        os.makedirs(dirpath, exist_ok=True)
        atomic_write_json(os.path.join(dirpath, "state.json"), state)
        self._reply(204)

    def do_GET(self):
        if self.path == "/healthz":
            return self._reply(200, b"ok")
        if self.path in ("/", "/dashboard"):
            page = render_dashboard().encode("utf-8")
            return self._reply(200, page, "text/html; charset=utf-8")
        if self.path.startswith("/install/"):
            rest = self.path[len("/install/"):]
            loc, _, query = rest.partition("?")
            kind = "dev" if "kind=dev" in query else "prod"
            install_id, _, sub = loc.partition("/")
            if not INSTALL_ID_RE.match(install_id):
                return self._reply(400, b"bad install id")
            install_id = install_id.lower()
            if sub == "raw":
                fname = "events.dev.jsonl" if kind == "dev" else "events.jsonl"
                path = os.path.join(ROOT, install_id, fname)
                if not os.path.exists(path):
                    return self._reply(404, b"no such install")
                size = os.path.getsize(path)
                self.send_response(200)
                self.send_header("Content-Type", "application/x-ndjson")
                self.send_header("Content-Length", str(size))
                self.send_header(
                    "Content-Disposition",
                    'attachment; filename="murmur-%s-%s.jsonl"' % (install_id[:8], kind),
                )
                self.end_headers()
                with open(path, "rb") as f:
                    while True:
                        chunk = f.read(1 << 20)
                        if not chunk:
                            break
                        self.wfile.write(chunk)
                return
            if sub:
                return self._reply(404)
            n = 200
            m = re.search(r"n=(\d+)", query)
            if m:
                n = int(m.group(1))
            page = render_install(install_id, kind, n)
            if page is None:
                return self._reply(404, b"no such install")
            return self._reply(200, page.encode("utf-8"), "text/html; charset=utf-8")
        self._reply(404)


def main():
    os.makedirs(ROOT, exist_ok=True)
    server = ThreadingHTTPServer(("127.0.0.1", 8600), Handler)
    sys.stderr.write("murmur-logs receiver on 127.0.0.1:8600\n")
    server.serve_forever()


if __name__ == "__main__":
    main()
