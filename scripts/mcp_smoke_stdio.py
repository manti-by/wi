#!/usr/bin/env python3
"""Drive a real MCP session over the binary's stdio transport and assert on it.

Complements mcp_smoke.py, which covers HTTP. Two transports means two
contracts: the tool surface must be identical across both, and stdout must
carry nothing but JSON-RPC.

smoke.sh hands both scripts the same fixture wiki, so the assertions here
mirror mcp_smoke.py's and the two transports can be compared line for line.

Usage: mcp_smoke_stdio.py <binary> <wiki-dir>
"""

import json
import queue
import subprocess
import sys
import tempfile
import threading
import time

PROTOCOL_VERSION = "2025-06-18"

# A server that accepts a request and never answers must fail this script, not
# hang it. Nothing upstream bounds the wait — neither the job nor smoke.sh — so
# an unbounded readline sits forever on a runner with no failure to show for it.
# Long enough that a loaded machine does not trip it, short enough that a real
# hang is reported in seconds rather than at the CI timeout.
CALL_TIMEOUT = 15

# A page from smoke.sh's fixture. Naming it is safe here precisely because the
# fixture is generated; naming one from the repository's own wiki is not.
PAGE = "2026-09-23-relay-mode"


class StdioSession:
    """One MCP session over a child process's stdin/stdout."""

    def __init__(self, binary, wiki, log):
        self.process = subprocess.Popen(
            [binary, "--transport", "stdio", "--wiki-path", wiki],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=log,
            text=True,
            bufsize=1,
        )
        self.next_id = 0

        # stdout is drained by one reader thread for the whole session, and the
        # lines it reads go on a queue in order. The main thread must be able to
        # stop waiting *while somebody is still reading the pipe*, which rules
        # out reading inline: a `select` on an already-buffered file object does
        # not see bytes that `readline` pulled ahead, so it either misses a
        # response or fires early. A thread has no such blind spot — it blocks
        # in the read, not in the wait. Daemon so it cannot outlive the script
        # if a failure kills the process first.
        self.lines = queue.Queue()
        self.reader = threading.Thread(target=self._drain_stdout, daemon=True)
        self.reader.start()

    def _drain_stdout(self):
        """Move every stdout line onto the queue, then mark EOF with None."""
        for line in iter(self.process.stdout.readline, ""):
            self.lines.put(line)
        self.lines.put(None)

    def send(self, message):
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()

    def call(self, method, params=None):
        self.next_id += 1
        request = {"jsonrpc": "2.0", "id": self.next_id, "method": method}
        if params is not None:
            request["params"] = params
        self.send(request)
        # One deadline for the whole wait, not per line: a response is one line,
        # so a per-line budget would only ever matter if the server dribbled
        # garbage, which the non-JSON branch below rejects anyway.
        deadline = time.monotonic() + CALL_TIMEOUT
        while True:
            remaining = deadline - time.monotonic()
            try:
                # A non-positive `remaining` raises Empty immediately, so an
                # already-expired deadline fails here rather than blocking.
                line = self.lines.get(timeout=remaining)
            except queue.Empty:
                self._kill(f"no response to {method} within {CALL_TIMEOUT}s")
            if line is None:
                raise SystemExit(f"smoke-stdio: stream closed during {method}")
            try:
                return json.loads(line)
            except json.JSONDecodeError:
                raise SystemExit(
                    f"smoke-stdio: stdout carried non-JSON during {method}: {line!r}\n"
                    "  stdout is the JSON-RPC channel; logs must go to stderr"
                )

    def _kill(self, reason):
        """Kill the child and fail. Used when the session is already unusable."""
        self.process.kill()
        # Reap it: a killed child that nobody waits on lingers as a zombie, and
        # this script is short enough that it is about to exit anyway.
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            pass
        raise SystemExit(f"smoke-stdio: {reason}")

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


def drive(binary, wiki, log):
    session = StdioSession(binary, wiki, log)

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
    for tool in tools["result"]["tools"]:
        expect(tool["inputSchema"]["type"] == "object", f"{tool['name']} schema is not an object")
    print("ok   tools/list")

    failed, text = session.tool("wiki_list_pages", {})
    expect(not failed, f"wiki_list_pages failed: {text}")
    expect(f"{PAGE}.md" in text, f"list is missing the relay page: {text}")
    print("ok   wiki_list_pages")

    failed, text = session.tool("wiki_search", {"query": "relay mode"})
    expect(not failed, f"wiki_search failed: {text}")
    expect(text.startswith(f"1. {PAGE}.md (score "), f"ranking is wrong: {text}")
    expect("   > L1: " in text, f"search hit has no snippets: {text}")
    print("ok   wiki_search ranks the relevant page first")

    failed, text = session.tool("wiki_get_page", {"name": PAGE})
    expect(not failed, f"wiki_get_page failed: {text}")
    expect(text.startswith("---\ntitle: Relay mode computed on read"), f"frontmatter is missing: {text}")
    print("ok   wiki_get_page returns the whole file")

    # The tool surface must be identical to the HTTP transport's, so an agent
    # cannot tell which one it was connected to.
    session.close()
    print("ok   exits cleanly when stdin closes")


def main():
    if len(sys.argv) != 3:
        raise SystemExit("usage: mcp_smoke_stdio.py <binary> <wiki-dir>")
    binary, wiki = sys.argv[1], sys.argv[2]

    # stderr goes to a file, not a pipe nobody reads: a pipe that fills up
    # blocks the server mid-session, and when the transport starts and never
    # answers, its log is the first thing worth seeing.
    with tempfile.TemporaryFile("w+") as log:
        try:
            drive(binary, wiki, log)
        except SystemExit as failure:
            log.seek(0)
            sys.stderr.write(f"--- {binary} stderr ---\n{log.read()}\n")
            raise failure

    print("smoke-stdio: all checks passed")


if __name__ == "__main__":
    main()