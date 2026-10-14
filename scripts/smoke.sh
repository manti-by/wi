#!/usr/bin/env bash
#
# End-to-end check that the built binary really speaks MCP.
#
# The unit tests cover parsing and ranking; nothing in `cargo test` can catch a
# transport that starts but never answers, so this starts the binary, drives a
# real Streamable-HTTP session through the three tools, and asserts on the text
# each one returns.

set -euo pipefail

port="${WI_SMOKE_PORT:-44321}"
repo_root=$(cd "$(dirname "$0")/.." && pwd)
binary="$repo_root/target/release/wi"

if [ ! -x "$binary" ]; then
    echo "smoke: $binary not found; run 'make build' first" >&2
    exit 1
fi

workdir=$(mktemp -d)
trap 'kill "${server_pid:-0}" 2>/dev/null || true; rm -rf "$workdir"' EXIT

# A fixture wiki with enough signal for the ranking to be distinguishable: one
# page about relay modes, one about something unrelated.
mkdir -p "$workdir/pages"
cat >"$workdir/pages/2026-09-23-relay-mode.md" <<'EOF'
---
title: Relay mode computed on read
date: '2026-09-23'
type: implementation
status: resolved
services: [consumer, redis_bus]
tags: [relay, mode]
---
# Relay mode computed on read

The relay mode is computed on read, not stored.
EOF

cat >"$workdir/pages/2026-07-16-fix-types.md" <<'EOF'
---
title: Fix type errors and verify CI flow
date: '2026-07-16'
type: debug
status: resolved
services: [ci]
tags: [ty]
---
# Fix type errors

The relay consumer had type errors in CI.
EOF

"$binary" --wiki-path "$workdir/pages" --port "$port" >"$workdir/server.log" 2>&1 &
server_pid=$!

# Wait for the listener rather than sleeping a fixed amount, so a slow runner
# does not produce a flaky failure.
for _ in $(seq 1 50); do
    if curl -fsS "http://127.0.0.1:$port/mcp" -X POST \
        -H 'Content-Type: application/json' \
        -H 'Accept: application/json, text/event-stream' \
        -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"smoke","version":"1"}}}' \
        >/dev/null 2>&1; then
        break
    fi
    sleep 0.2
done

python3 "$repo_root/scripts/mcp_smoke.py" "http://127.0.0.1:$port/mcp"