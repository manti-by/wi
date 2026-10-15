---
title: 'Wiki tooling and AGENTS.md'
date: '2026-10-15'
type: implementation
status: resolved
session_id: ses_ec457decfffejIVnnqkJODH0kp
services: []
branch: feature/mnt-238-openwiki-mcp-unified-binary
tickets: [MNT-238]
tags: [wiki, skills, agents, tooling, ci, dependencies]
related: [2026-10-14-installer-and-release-pipeline.md, 2026-10-14-wi-mcp-server.md, 2026-10-14-differential-testing-parity.md]
---
# Wiki tooling and AGENTS.md

## TL;DR

The project gained the standard wiki + agent-skill setup: `sk.toml` pinning six
skills, the four wiki meta files, and an `AGENTS.md`. Wiring it in surfaced two
real defects — an unused `tower` dependency and a release gate that ignored
`sk.toml` — and one live dependency concern: **`serde_yaml` is deprecated and
unmaintained**, and the frontmatter parser depends on its exact behaviour.

---

## Step 1 — Reconcile the existing pages with the new conventions

**File:** `wiki/README.md`, `wiki/TEMPLATE.md`

The three pages from the MNT-238 session predate these conventions. Per-page gaps:

- No `session_id`, `services` or `branch` in frontmatter.
- Body cross-links used GFM (`[text](file.md)`) rather than the `[[wikilink]]`
  form the README mandates.
- `related:` listed bare slugs; `TEMPLATE.md` lists filenames with `.md`.
- No `## Follow-ups` or `## References` sections.

`session_id` is set to `-` on all three. They were written before session ids were
recorded, and inventing one would misattribute them to whichever session happens
to touch them next — the wiki-sync skill keys its page lookup on exactly that
field.

## Step 2 — Fix the index

**File:** `wiki/INDEX.md`

The index had two entries collapsed onto one line (a missing newline had merged
the ranking and installer pages into a single bullet), no dates, and no
`_Newest first._` marker that the wiki-sync skill anchors new entries against.
Rewritten in the prescribed format:

```
- [<title>](pages/<filename>) — <one-line summary> (<date>)
```

## Step 3 — Wire `sk.toml` into the release gate

**File:** `.github/scripts/detect-version.sh`

`sk` asserts that `project.version` in `sk.toml` matches `version` in its own
manifest, precisely so a release cannot go out whose tag disagrees with its own
metadata. This port's gate only read `Cargo.toml`, because `sk.toml` did not
exist when it was written. Now that it does, the cross-check is restored:

- Versions agree → normal diff-based bump detection.
- `sk.toml` drifts → the job **fails**, naming both versions.
- `workflow_dispatch` → always publishes, so a failed release can be retried.

All three paths were exercised against a scratch clone rather than assumed.

## Step 4 — Two defects found while gathering facts

**File:** `Cargo.toml` — `tower` was a direct dependency with no use anywhere in
`src/`. It predates the switch from a hand-rolled tower service to
`axum::Router` in `src/main.rs`, and was left behind. Removed.

**File:** `Cargo.toml` — the crate `description` still read "OpenWiki MCP server"
after the project was renamed to *WI Tool*. **Not fixed** — corrected on
2026-10-15, because this page previously said "Aligned" and no such edit was
made: `Cargo.toml`'s only change in that session was removing `tower`, and
`git log -p --follow -- Cargo.toml` shows the `description` line was introduced
once in `2b58428` and never touched since. It still reads
`"OpenWiki MCP server: search and read Markdown wiki pages over MCP"`. Whether
that is a defect depends on whether the product is now "WI Tool" or still
OpenWiki's MCP server, which is why it is filed as Q-002 rather than fixed
here.

## Update — 2026-10-15 08:20

### stdio becomes the default transport

`wi` served Streamable HTTP only, because that is what MNT-238 specified. But
HTTP is the wrong default for the client that matters: MCP clients launch a
local server as a child process and speak to it over stdin/stdout. That needs no
port, no socket, and no `Host` header, and it means two `wi` instances cannot
collide.

**File:** `src/main.rs`, `Cargo.toml`

Added `--transport stdio|http`, defaulting to `stdio`. HTTP is unchanged behind
the flag, including the `Host` allowlist coupling. `--host`, `--port` and
`--allowed-host` are documented as HTTP-only rather than rejected, since silently
ignoring a flag the user typed is its own kind of confusion.

```console
$ wi                                 # stdio
$ wi --transport http --port 4321    # Streamable HTTP
$ echo | wi                         # exits 1: never spoke MCP
```

Two details worth keeping:

- **stdout is the JSON-RPC channel.** `init_tracing` writes to stderr for that
  reason; one `println!` corrupts the stream and the client drops the session.
  The stdio smoke test asserts it, failing on any non-JSON line on stdout.
- **`--wiki-path` should be absolute in stdio mode.** The client chooses the
  working directory, so the relative default `wiki/pages` can resolve somewhere
  unexpected. Documented rather than changed, since a different default per
  transport would be a worse surprise.

`rmcp`'s stdio transport sits behind the `transport-io` feature, so the
dependency gained it.

The default flip broke one thing nobody asked about: `make run` is
`cargo run -- $(ARGS)` with no `ARGS`, so it inherited stdio and a contributor
typing `make run` got a process blocking on stdin instead of a listener. The
target now defaults `ARGS` to `--transport http`, which is what the target has
always been for; `make run ARGS="--transport stdio"` still gets the default.

### CI was not running the tests

**File:** `Makefile`, `.github/workflows/ci.yml`, `.github/workflows/release.yml`

Commit `e6fd2d5` renamed `fmt-check` to `check`, so `make check` became
formatting-only and a new `ci` target took over the old full-gate role. Both
workflows still invoked `make check` — under a step named *"cargo fmt, clippy and
tests"*. **Nothing in CI was running `cargo test` or `clippy`.** The 77 unit
tests, and the parity guarantees in them, were not enforced on any push.

`make lint` was broken too: it depended on `fmt-check`, which no longer existed.

Both fixed — workflows call `make ci`, `lint` depends on `check`. The naming trap
is recorded in AGENTS.md so it is not reintroduced: when a workflow step's name
and the target it invokes disagree, one of them is lying.

### `make smoke` now covers both transports

**File:** `scripts/smoke.sh`, `scripts/mcp_smoke_stdio.py`

One transport was tested while two existed, which is exactly how a transport can
start and never answer without anything noticing. The stdio half spawns the real
binary, drives a full session through the same three tools, and pins the exit
status — closing stdin after a completed handshake must exit `0`, since a client
closing its pipe on quit would otherwise see a spurious failure.

**Both halves now share one fixture wiki** rather than the stdio half reading
this repository's own pages. Two reasons: the assertions in the two scripts
become directly comparable, which is the parity claim being made, and the smoke
test stops failing whenever a wiki page is archived or renamed — `wiki-archive`,
`wiki-sync` and `wiki-dedup` all move pages, and a release should not be blocked
by an edit to a document. The fixture is generated by `smoke.sh`, so naming a
page from it is safe.

The stdio script sends the child's stderr to a file rather than a pipe nobody
reads. A pipe nobody reads deadlocks the server mid-session once the 64 KB
buffer fills, and the log is exactly what you want when a transport starts and
never answers, so the file is dumped on failure.

## Follow-ups

- **`serde_yaml` is deprecated and unmaintained** (0.9.34+deprecated; the author
  has archived the repository). `serde_yml` is *also* deprecated — it is a
  compatibility shim forwarding to `noyalib` — so there is no drop-in
  successor. `src/page.rs` depends on this crate more deeply than it looks: the
  whole `MetaValue` deserializer relies on `serde_yaml`'s `deserialize_any`
  handing mapping entries to a visitor that can overwrite a duplicate key, which
  is what makes repeated frontmatter keys resolve last-wins instead of dropping
  the page. Any migration must preserve that, and the parity harness in
  [[2026-10-14-differential-testing-parity]] is what would prove it.
- No decision has been taken on a replacement. Worth revisiting once one settles.

## References

- Related: [[2026-10-14-installer-and-release-pipeline]],
  [[2026-10-14-wi-mcp-server]], [[2026-10-14-differential-testing-parity]]
- Code: `sk.toml`, `sk.lock`, `AGENTS.md`, `.github/scripts/detect-version.sh`
- External: https://github.com/dtolnay/serde-yaml (archived)