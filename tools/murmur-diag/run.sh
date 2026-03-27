#!/usr/bin/env bash
# Bootstrap wrapper: creates a venv and installs deps before running the MCP server.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
VENV="$DIR/.venv"

if [ ! -d "$VENV" ]; then
    python3 -m venv "$VENV" >&2
fi

if ! "$VENV/bin/python" -c "import mcp" 2>/dev/null; then
    "$VENV/bin/pip" install -q -r "$DIR/requirements.txt" >&2
fi

exec "$VENV/bin/python" "$DIR/server.py" "$@"
