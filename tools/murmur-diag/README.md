# murmur-diag MCP server

`murmur-diag` is a local, read-only MCP server for inspecting Murmur telemetry.
It reads files already written on this Mac and uses MCP's stdio transport. It
does not upload logs, open a network port, or modify Murmur data.

## Tools

| Tool | Purpose | Main arguments |
|------|---------|----------------|
| `check_health` | Snapshot the latest keyboard, recording, warning/error, listener, uptime, and possibly-stuck state. | None |
| `query_events` | Filter structured events and paginate the result. | `stream`, `level`, `since`, `until`, `pattern`, `limit`, `offset` |
| `search_logs` | Regex-search human-readable logs with optional surrounding lines. | `pattern`, `since`, `until`, `context`, `limit` |
| `session_summary` | Summarize app sessions, recordings, keyboard activity, warnings/errors, missed hotkeys, and peak RSS. | `since`, `limit` |
| `correlate_keyboard` | Check whether keyboard start events led to native recording starts within a time window. | `since`, `until`, `max_gap_ms` |

Times accept ISO 8601 timestamps or relative values such as `30m`, `24h`, and
`7d`. Event query `pattern` and log search `pattern` are case-insensitive regular
expressions.

## Log sources

By default the server reads:

```text
~/Library/Application Support/local-dictation/logs/
```

Both Murmur build identities write to that same legacy-named directory. They
use separate files, and `murmur-diag` reads current and rotated variants of all
four:

| Build | Bundle ID | Structured events | Human-readable log |
|-------|-----------|-------------------|--------------------|
| Release | `com.localdictation` | `events.jsonl*` | `app.log*` |
| Dev/debug | `com.localdictation.dev` | `events.dev.jsonl*` | `app.dev.log*` |

Results identify their origin. Structured events include `diag_source` with
`build` and `file` fields; log matches include `source` and `file`. Session and
keyboard correlation analysis keeps dev and release events separate so two apps
running at once cannot be correlated into a false success.

For a fixture or non-default local directory, set `MURMUR_LOG_DIR` when
registering or launching the server. This does not change Murmur's write path.

## Setup and direct launch

The only declared dependency is the Python MCP SDK in `requirements.txt`.
Launch with:

```bash
/absolute/path/to/murmur-app/tools/murmur-diag/run.sh
```

`run.sh` creates `tools/murmur-diag/.venv` on first use, installs
`requirements.txt` if the `mcp` module is missing, and then starts `server.py`
over stdio. The `.venv` is local and gitignored. Running the command directly
will wait for an MCP client on stdin; use `Ctrl-C` to stop it.

Run the focused tests with:

```bash
tools/murmur-diag/.venv/bin/python -m unittest discover \
  -s tools/murmur-diag -p 'test_*.py' -v
```

## Register once, not once per worktree

Use one stable checkout as the shared installation and register its absolute
`run.sh` path in the MCP client's **user-level** configuration. Do not add the
server to a worktree's `.mcp.json` or `.codex/config.toml`; project-level
registration is copied into every worktree and causes redundant readers of the
same shared log directory.

Codex registration:

```bash
codex mcp remove murmur-diag 2>/dev/null || true
codex mcp add murmur-diag -- bash \
  /absolute/path/to/murmur-app/tools/murmur-diag/run.sh
codex mcp get murmur-diag
```

Claude Code registration:

```bash
claude mcp remove --scope user murmur-diag 2>/dev/null || true
claude mcp add --scope user murmur-diag -- bash \
  /absolute/path/to/murmur-app/tools/murmur-diag/run.sh
```

For another MCP client, use the equivalent user-level JSON entry:

```json
{
  "mcpServers": {
    "murmur-diag": {
      "type": "stdio",
      "command": "bash",
      "args": ["/absolute/path/to/murmur-app/tools/murmur-diag/run.sh"]
    }
  }
}
```

The registration is shared, while stdio process lifetime still belongs to the
client: each simultaneously active MCP client starts its own child process.
Making one process serve multiple clients would require a socket or network
daemon, which this privacy-first local tool intentionally does not add.

After changing the stable checkout path, update the single user-level
registration. Worktrees need no MCP configuration changes.
