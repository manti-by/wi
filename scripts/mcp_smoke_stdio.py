#!/usr/bin/env python3
"""Drive a real MCP session over the binary's stdio transport and assert on it.

Complements mcp_smoke.py, which covers HTTP. Two transports means two
contracts: the tool surface must be identical across both, and stdout must
carry nothing but JSON-RPC.
"""

import json
import subprocess
import sys

BINARY = sys.argv[1] if len(sys.argv) > 1 else "target/release/wi"
WIKI = sys.argv[2] if len(sys.argv) > 2 else "wiki/pages"
PROTOCOL_VERSION = "2025-06-18"


class StdioSession:
    """One MCP session over a child process's stdin/stdout."""

    def __init__(self):
        self.process = subprocess.Popen(
            [BINARY, "--transport", "stdio", "--wiki-path", WIKI],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        self.next_id = 0

    def send(self, message):
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()

    def call(self, method, params=None):
        self.next_id += 1
        request = {"jsonrpc": "2.0", "id": self.next_id, "method": method}
        if params is not None:
            request["params"] = params
        self.send(request)
        while True:
            line = self.process.stdout.readline()
            if not line:
                raise SystemExit(f"smoke-stdio: stream closed during {method}")
            try:
                return json.loads(line)
            except json.JSONDecodeError:
                raise SystemExit(
                    f"smoke-stdio: stdout carried non-JSON during {method}: {line!r}\n"
                    "  stdout is the JSON-RPC channel; logs must go to stderr"
                )

    def notify(self, method, params=None):
        message = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            message["params"] = params
        self.send(message)

    def tool(self, name, arguments):
        reply = self.call("tools/call", {"name": name, "arguments": arguments})
        if "error" in reply:
            raise SystemExit(f"smoke-stdio: {name} returned a protocol error: {reply['error']}")
        result = reply["result"]
        return result.get("isError", False), result["content"][0]["text"]

    def close(self):
        try:
            self.process.stdin.close()
        except OSError:
            pass
        try:
            status = self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()
            raise SystemExit("smoke-stdio: server did not exit when stdin closed")
        # Closing stdin after a completed handshake is an ordinary shutdown, so
        # the exit status has to be 0. A client that closes the pipe on quit
        # would otherwise see a spurious failure.
        expect(status == 0, f"exit status {status} after stdin closed, expected 0")
        return status


def expect(condition, message):
    if not condition:
        raise SystemExit(f"smoke-stdio: {message}")


session = StdioSession()
reply = session.call(
    "initialize",
    {
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": {"name": "smoke", "version": "1"},
    },
)
expect(reply["result"]["serverInfo"]["name"] == "wi", "server did not identify as wi")
print("ok   initialize")

session.notify("notifications/initialized")

tools = session.call("tools/list")
names = [tool["name"] for tool in tools["result"]["tools"]]
expect(
    names == ["wiki_search", "wiki_get_page", "wiki_list_pages"],
    f"unexpected tool list: {names}",
)
print("ok   tools/list")

failed, text = session.tool("wiki_list_pages", {})
expect(not failed, f"wiki_list_pages failed: {text}")
expect("2026-10-14-wi-mcp-server.md" in text, f"list is missing the repo's own page: {text}")
print("ok   wiki_list_pages")

failed, text = session.tool("wiki_search", {"query": "relay mode computed on read"})
expect(not failed, f"wiki_search failed: {text}")
expect(text.startswith("1. "), f"search returned no ranked hits: {text}")
expect("   > L" in text, f"search hit has no snippets: {text}")
print("ok   wiki_search ranks and snippets")

failed, text = session.tool("wiki_get_page", {"name": "2026-10-14-wi-mcp-server"})
expect(not failed, f"wiki_get_page failed: {text}")
expect(text.startswith("---\n"), "page body lost its frontmatter")
print("ok   wiki_get_page")

# The tool surface must be identical to the HTTP transport's, so an agent
# cannot tell which one it was connected to.
session.close()
print("ok   exits cleanly when stdin closes")
print("smoke-stdio: all checks passed")