---
title: 'stdio transport PR review'
date: '2026-10-15'
type: code-review
status: resolved
session_id: ses_ec13a098fffeOS9QGLVc135n1T
services: []
branch: stdio-support
tickets: [MNT-238]
tags: [mcp, stdio, code-review, ci, testing, docs]
related: [2026-10-15-wiki-tooling-and-agents-file.md, 2026-10-14-wi-mcp-server.md, 2026-10-14-installer-and-release-pipeline.md, 2026-10-14-differential-testing-parity.md]
---

# stdio transport PR review

## TL;DR

Reviewed PR#1, which made stdio the default MCP transport and repaired a CI gate
that had stopped running tests, then fixed all nine findings. **The Rust itself
was sound** — `make ci` passed and both transports spoke MCP on first run, so
every finding was documentation, test-fixture or ergonomics rather than a defect
in the transport wiring. The two that mattered: `make run` silently inherited the
new default and stopped handing a contributor a listener, and the stdio smoke
test asserted on a page in this repo's own wiki, so archiving or renaming one
document would fail the release job.

---

## Findings

### 1. `make run` inherited the new default transport

- **File:** `Makefile:9` (before)
- **Severity:** Medium — dev-experience regression, silently broken

The target was `cargo run -- $(ARGS)` with no default for `ARGS`, so flipping
the binary's default from HTTP to stdio turned `make run` into a process that
blocks reading stdin. Interactively it printed one INFO line and appeared to
hang; non-interactively it exited 1 at EOF. Either way it no longer did the one
thing the target is named for.

This is the generalisable part: **a default flip is not confined to the flag
that changed.** Every entry point that inherits the default inherits the flip,
and only the one that was reviewed gets updated.

- **Fix:** `Makefile:11` now sets `ARGS ?= --transport http` above the target,
  so `make run` gives a listener again and `make run ARGS="--transport stdio"`
  still reaches the default. Verified: `make run` answers `http=200` on
  `:4321/mcp`.

### 2. The stdio smoke test asserted on a mutable wiki page

- **File:** `scripts/mcp_smoke_stdio.py:113` (before)
- **Severity:** Medium — CI and the release gate depend on document content

The stdio half ran against `$repo_root/wiki/pages` and hardcoded
`2026-10-14-wi-mcp-server.md`. That page can move: `wiki-archive` relocates
pages to `wiki/archive/`, and `wiki-sync`, `wiki-consistency` and `wiki-dedup`
all rewrite them. At which point `make smoke` fails inside the release
workflow's `mcp` job, blocking a release over an edit to a document.

- **Fix:** `scripts/smoke.sh` now builds its fixture wiki once, before either
  half, and both transports read the same directory. `scripts/mcp_smoke_stdio.py:23`
  names a page from that generated fixture instead. Two payoffs beyond the
  removal of the coupling: the assertions in the two scripts now mirror each
  other, which is the parity claim the file's own docstring makes, and the text
  each tool returns is directly comparable between transports.

### 3. A doc comment became false without being touched

- **File:** `src/main.rs:301` (before)
- **Severity:** Low as prose, high as leverage

```rust
/// stdout and stderr are both ordinary streams here rather than a JSON-RPC
/// channel, but stderr is still the right home for logs: ...
```

True while HTTP was the only transport. The PR added the correct statement to
`AGENTS.md` and left this one in the source file, where the person most likely
to add a `println!` will read it. A behaviour change does not edit the comments
that describe it.

- **Fix:** the comment now states that stdout is the JSON-RPC channel and that
  the explicit `std::io::stderr` is load-bearing because the subscriber's
  default writer is stdout.

### 4. `ServeError::Transport` discarded its source chain

- **File:** `src/error.rs:39` (before)
- **Severity:** Low

`Transport(String)` with `.map_err(|e| …(e.to_string()))` flattened rmcp's
`ServerInitializeError`, while every neighbouring variant keeps a typed
`#[source]`.

- **Fix:** `#[source] Box<dyn std::error::Error + Send + Sync>`, matching the
  `Logging` variant already in the file. Worth knowing: thiserror plus anyhow
  prints the source chain, so `{0}` in the message *and* `#[source]` renders the
  text twice —

  ```text
  Error: stdio transport failed: connection closed: initialize request

  Caused by:
      connection closed: initialize request
  ```

  That duplication is the existing convention here, not a new artifact: `Bind`
  and `Serve` already render the same way.

### 5. An undrained pipe could deadlock the smoke harness

- **File:** `scripts/mcp_smoke_stdio.py:26` (before)
- **Severity:** Low — latent, not currently reachable

`stderr=subprocess.PIPE` with nobody reading it. The server writes two INFO
lines, nowhere near the 64 KB buffer, so it cannot hang today — but it would
block mid-session rather than fail, which is the worst failure mode for a test
whose purpose is diagnosing a server that starts and never answers.

- **Fix:** `stderr` goes to a `tempfile.TemporaryFile`, dumped to stderr when an
  assertion fails. Verified by pointing the script at an empty wiki: the
  failure prints the server's own log, including the completed handshake.

### 6. README duplicated a sentence across a section boundary

- **File:** `README.md:76` (before)
- **Severity:** Low

The new `### http` section ended on "It binds loopback by default, because it
serves a project's private notes." and the pre-existing paragraph beneath it
opened with the same sentence.

- **Fix:** dropped the added line and rewrapped the paragraph.

### 7. The pasted `--help` output had drifted from the real CLI

- **File:** `README.md:35` (before)
- **Severity:** Low

The block was hand-maintained: it showed the old `USAGE:`/`OPTIONS:` layout,
omitted the `Possible values` list clap emits for the new `value_enum`, and
omitted the `long_about` this PR rewrote. Editing it by hand again would push
it further out of date.

- **Fix:** replaced with verbatim `wi -h` output — the short form is compact
  enough to paste — plus a note to regenerate or drop it rather than let it
  drift.

### 8. Two README examples no longer bound anything

- **File:** `README.md:82`, `README.md:94`
- **Severity:** Medium in effect, Low to fix — found while fixing #6

```console
$ wi --host 0.0.0.0 --port 8080 --wiki-path ~/notes/wiki
```

Both the wildcard-bind example and the `--allowed-host` example omitted
`--transport http`. Under the new default they start stdio and bind nothing, so
the one place a reader learns the DNS-rebinding guard is a command that does
not reach the feature it illustrates.

- **Fix:** `--transport http` added to both.

### 9. Stale references left behind by the `make check` repair

- **File:** `Makefile:4`, `Makefile:34`, `.github/workflows/ci.yml:45`
- **Severity:** Low

The PR removed the last reference to `fmt-check` but left it in `.PHONY`; the
`smoke` help line and the `mcp` job's comment both still described one
transport while `README.md` and `AGENTS.md` were updated to say both.

- **Fix:** `.PHONY` drops `fmt-check`; both comments say "both transports".

### 10. The default flip is a breaking change with nowhere to be recorded

- **File:** `.github/scripts/publish.sh:27`
- **Severity:** Informational

Anyone invoking `wi` and expecting a listener on `:4321` now gets a process that
reads stdin and binds nothing, with no error. There is no `CHANGELOG.md` in
this repo and none should be invented: `publish.sh` passes `--generate-notes`,
so GitHub composes release notes from the commit and PR history — which is why
the PR's own description matters and is worth writing well. The migration path
is documented in `README.md`'s `stdio` / `http` sections.

## Summary

Findings by severity, since a table does not diff cleanly (see the `wiki-sync`
skill's no-markdown-tables rule, which overrides the `code-review` preset's
summary table in `TEMPLATE.md`):

- **1 — `Makefile:9`** — `make run` inherited the stdio default and stopped
  binding. Fixed with `ARGS ?= --transport http`.
- **2 — `scripts/mcp_smoke_stdio.py:113`** — smoke test coupled to a mutable
  wiki page; archiving a document would fail the release job. Fixed by sharing
  one generated fixture with the HTTP half.
- **8 — `README.md:82`, `README.md:94`** — HTTP examples missing
  `--transport http`, so they bound nothing. Fixed.
- **3 — `src/main.rs:301`** — doc comment asserting stdout is not the JSON-RPC
  channel. Fixed.
- **4 — `src/error.rs:39`** — `Transport(String)` dropped the source chain.
  Fixed.
- **5 — `scripts/mcp_smoke_stdio.py:26`** — undrained `stderr` pipe, latent
  deadlock. Fixed.
- **6 — `README.md:76`** — sentence duplicated across a section break. Fixed.
- **7 — `README.md:35`** — hand-maintained help block had drifted. Fixed.
- **9 — `Makefile:4`, `Makefile:34`, `ci.yml:45`** — stale references to a
  removed target and to a single transport. Fixed.
- **10 — `publish.sh`** — breaking default change with no changelog by design;
  GitHub generates the notes. No action.

Nothing in `src/search.rs`, `src/page.rs`, `src/corpus.rs`, `src/tools.rs` or
`src/config.rs` was touched, so no ranking or parsing behaviour moved — which
matters because `AGENTS.md` holds that a cosmetic-looking change to the analyzer
changes which pages match.

## Verification

- `make ci` — fmt, `clippy --all-targets` and 77 unit tests, all green.
- `make smoke` — both transports green against the shared fixture; the stdio
  half asserts the same ranking text as the HTTP half.
- `make run` — answers `http=200` on `127.0.0.1:4321/mcp`.
- `echo | wi` — still exits 1 with `stdio transport failed: connection closed:
  initialize request`, so [[2026-10-15-wiki-tooling-and-agents-file]]'s claim
  still holds after the error type changed.
- `bash -n scripts/smoke.sh`, `.github/scripts/lint-shell.sh`,
  `python3 -m py_compile` — clean.
- Failure path of the stdio script exercised deliberately against an empty wiki.

---

## Follow-ups

- **An uncommitted `0.1.0` → `0.2.0` bump appeared in `Cargo.toml` and
  `sk.toml` mid-session and is not this session's work.** It was left in place.
  Per [[2026-10-14-installer-and-release-pipeline]], merging it to `master`
  triggers a release, and the `mcp` CI job builds `--locked`, so `Cargo.lock`
  must be committed in the same change — `detect-version.sh` will otherwise fail
  the release. A human should confirm the bump is intended.
- The `wiki/README.md` mapping-filename discrepancy is **Q-001** in
  `QUESTIONS.md` and is awaiting a human answer, so `wiki/README.md` was
  deliberately not touched even though this session verified the deployed name
  is `.sessions.json`.
- The parity harness described in
  [[2026-10-14-differential-testing-parity]] is still unbuilt; this session
  reviewed transport wiring and could not check the Python reference.

## References

- Related: [[2026-10-15-wiki-tooling-and-agents-file]], [[2026-10-14-wi-mcp-server]],
  [[2026-10-14-installer-and-release-pipeline]], [[2026-10-14-differential-testing-parity]]
- External: <https://github.com/manti-by/wi/pull/1>