#!/usr/bin/env python3
"""Drive a real MCP session against a running wi and assert on its output.

Kept in Python because it is client plumbing, not product code: the point is to
exercise the server the way an agent client does — initialize, notify, list,
call — rather than to reimplement anything about search.

Usage: mcp_smoke.py <endpoint-url>
"""

import json
import sys
import urllib.error
import urllib.request

URL = sys.argv[1] if len(sys.argv) > 1 else "http://127.0.0.1:4321/mcp"
PROTOCOL_VERSION = "2025-06-18"


def post(payload, session=None):
    """Posts one JSON-RPC message and returns (session_id, last response)."""
    request = urllib.request.Request(URL, data=json.dumps(payload).encode(), method="POST")
    request.add_header("Content-Type", "application/json")
    request.add_header("Accept", "application/json, text/event-stream")
    if session:
        request.add_header("Mcp-Session-Id", session)
    try:
        with urllib.request.urlopen(request) as response:
            session_id = response.headers.get("Mcp-Session-Id")
            body = response.read().decode()
    except urllib.error.HTTPError as error:
        raise SystemExit(f"smoke: HTTP {error.code} for {payload['method']}: {error.read().decode()}")

    # Streamable HTTP replies as SSE, whose priming events carry no payload.
    messages = [
        json.loads(line[6:])
        for line in body.splitlines()
        if line.startswith("data: ") and line[6:].strip()
    ]
    return session_id, (messages[-1] if messages else None)


def call(session, request_id, name, arguments):
    """Calls a tool and returns (is_error, text)."""
    _, reply = post(
        {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        },
        session,
    )
    if "error" in reply:
        raise SystemExit(f"smoke: {name} returned a protocol error: {reply['error']}")
    result = reply["result"]
    return result.get("isError", False), result["content"][0]["text"]


def expect(condition, message):
    if not condition:
        raise SystemExit(f"smoke: {message}")


session, reply = post(
    {
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "smoke", "version": "1"},
        },
    }
)
expect(session is not None, "initialize did not return a session id")
expect(reply["result"]["serverInfo"]["name"] == "wi", "server did not identify as wi")
print("ok   initialize")

post({"jsonrpc": "2.0", "method": "notifications/initialized"}, session)

_, reply = post({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}, session)
names = [tool["name"] for tool in reply["result"]["tools"]]
expect(
    names == ["wiki_search", "wiki_get_page", "wiki_list_pages"],
    f"unexpected tool list: {names}",
)
for tool in reply["result"]["tools"]:
    expect(tool["inputSchema"]["type"] == "object", f"{tool['name']} schema is not an object")
print("ok   tools/list")

failed, text = call(session, 10, "wiki_list_pages", {})
expect(not failed, f"wiki_list_pages failed: {text}")
expect("2026-09-23-relay-mode.md" in text, f"list is missing the relay page: {text}")
expect(text.index("2026-07-16") < text.index("2026-09-23"), "list is not in file-name order")
print("ok   wiki_list_pages")

failed, text = call(session, 11, "wiki_search", {"query": "relay mode"})
expect(not failed, f"wiki_search failed: {text}")
expect(text.startswith("1. 2026-09-23-relay-mode.md (score "), f"ranking is wrong: {text}")
expect("   > L1: " in text, f"search hit has no snippets: {text}")
print("ok   wiki_search ranks the relevant page first")

# A stop-word-only query is an empty result, not an error.
failed, text = call(session, 12, "wiki_search", {"query": "the of and"})
expect(not failed, f"a stop-word query should not be an error: {text}")
expect("No matching wiki pages" in text, f"unexpected text: {text}")
print("ok   wiki_search reports no matches without erroring")

# limit must actually truncate the ranked list.
_, unlimited = call(session, 13, "wiki_search", {"query": "relay"})
_, limited = call(session, 14, "wiki_search", {"query": "relay", "limit": 1})
expect(unlimited.count("\n1. ") == 0 and "\n2. " in unlimited, "expected two ranked hits")
expect("\n2. " not in limited, "limit=1 was not applied")
print("ok   wiki_search honours limit")

failed, text = call(session, 15, "wiki_get_page", {"name": "2026-09-23-relay-mode"})
expect(not failed, f"wiki_get_page failed: {text}")
expect(text.startswith("---\ntitle: Relay mode computed on read"), f"frontmatter is missing: {text}")
print("ok   wiki_get_page returns the whole file")

# Traversal must not escape the pages directory.
failed, text = call(session, 16, "wiki_get_page", {"name": "../../etc/passwd"})
expect(failed, "path traversal was allowed")
expect("path outside wiki directory" in text, f"unexpected error text: {text}")
print("ok   wiki_get_page rejects traversal")

# Validation failures are tool errors the agent can read.
for request_id, (name, arguments), expected in [
    (17, ("wiki_search", {}), "query is required"),
    (18, ("wiki_search", {"query": "x" * 501}), "exceeds maximum length"),
    (19, ("wiki_get_page", {}), "name is required"),
    (20, ("wiki_nope", {}), "Unknown tool"),
]:
    failed, text = call(session, request_id, name, arguments)
    expect(failed, f"{name} {arguments} should have been an error")
    expect(expected in text, f"unexpected error text for {name}: {text}")
print("ok   validation errors are readable tool errors")

print("smoke: all checks passed")