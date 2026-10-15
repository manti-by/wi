---
title: 'Differential testing against the Python original'
date: '2026-10-14'
type: investigation
status: resolved
session_id: '-'
services: []
branch: feature/mnt-238-openwiki-mcp-unified-binary
tickets: [MNT-238]
tags: [testing, porting, parity, methodology, bm25]
related: [2026-10-14-wi-mcp-server.md, 2026-10-14-wiki-search-ranking.md, 2026-10-15-wiki-tooling-and-agents-file.md]
---
# Differential testing against the Python original

## Net effect

`wi` is a port of the Python wiki tools in
[odin#41](https://github.com/manti-by/odin/pull/41), and **every substantive bug
found in it was silent** — no error, no log line, no failed test. Not one was
caught by a unit test written alongside the code. All of them were caught by
running the Rust and the Python on the *same inputs* and diffing.

Three review rounds produced: a wrong tokenizer, a wrong frontmatter delimiter, a
security hole in the `--host` flag, a page-dropping frontmatter bug in two
different forms, and a flaky test suite that would have made the release gate red
one run in four.

---

## Why unit tests did not catch it

A test written next to a reimplementation asserts the author's *model* of the
original, not the original. When the model is wrong the test is wrong in exactly
the same way, and passes.

The tokenizer is the cleanest example. The reference analyzer is
`re.findall(r"[a-z0-9][a-z0-9_.\-]*", text.lower())`. A reasonable reading is
"a run of word characters, allowing `.`/`_`/`-` inside" — which is what was
implemented, with a trailing separator excluded because a trailing dot looks like
punctuation. The regex does the opposite: once a term starts, `_`, `.` and `-`
continue it unconditionally, so `release.` and `self._private` are single terms.

The unit test asserted `extract_terms("relay, mode.") == ["relay", "mode"]`. That
test was *pinning the bug*.

## What made the difference

Three properties of the differential setup, in order of value:

**1. Same inputs, both implementations.** The Python original was fetched from
the repo (`gh api .../contents/<path> -q .content | base64 -d`) and run in the
same harness as the Rust — for the analyzer, a regex in Python; for the page
format, PyYAML; for the tools, an MCP session driven over HTTP. When two answers
disagreed, one of them was a bug, and which one was usually not obvious.

**2. Fuzzing the functions, not the binary.** Thousands of generated inputs
through one function each. The three verified-clean classes:

- **Frontmatter delimiter** — 37,102 inputs over whitespace runs, fence
  variations, `\x0b`/`\x0c`/`\x1c`/`\x85`/`\xa0`/`\u2028`, unicode and CRLF.
  Zero mismatches in block or body.
- **Analyzer** — 40,000 random inputs plus an exhaustive sweep of every code
  point whose `str.lower()` yields ASCII or expands (U+0130, U+1E9E, U+01C5,
  final sigma, U+212A, U+FB03 …). Zero mismatches, which also established that
  Rust's `to_lowercase` matches Python's `str.lower` for this purpose.
- **Line splitting** — 60,000 inputs. Zero mismatches in snippet line numbers,
  ranking, cap or truncation.

**3. Scoring end-to-end, not just outputs.** The first round compared scores and
snippets on the project's own three wiki pages. Query `mcp` returned `2.035` from
Python and `2.044` from Rust — a difference no output-shape assertion would have
caught, and one that traced directly to the tokenizer. Because the analyzer feeds
the corpus-wide field lengths, which feed the length-normalization divisor, a
divergence that looks cosmetic in one function changes *which pages match*.

## Where the approach found things reading did not

Two findings no amount of code review surfaced, because both were only observable
by running the two side by side:

- **The `--host` security hole.** `--host 0.0.0.0` widened the bind but left the
  transport's `Host` allowlist loopback-only. The server was simultaneously
  reachable from the network by spoofing `Host: localhost`, and unreachable by
  the address the operator actually asked for. Nothing in `src/main.rs` looks
  wrong; the bug is in the *interaction* with a default three files away.
- **The flaky test suite.** Two tests keyed their fixture directory on the
  process id and a file name, so two tests that both wanted `bad.md` truncated
  each other's file mid-read. Measured at 6 failures in 15 runs — on the gate
  that publishes releases.

## The cost of getting a heuristic wrong

The duplicate-key fix took three attempts, and the two failures are the most
instructive part of the port:

1. Matching on the parser's error text. Never fired for a nested repeat, because
   the message is prefixed with the key path.
2. Rewriting the frontmatter block textually. Dropped only the key line, orphaning
   an indented value; deleted lines belonging to *unrelated* keys when a
   multi-line flow value threw off the computed scope; and was cubic in nesting
   depth — 0.03s at 100 levels, ~450s at 3200, while holding the cache lock.
   A fuzz of 6,000 duplicate-key shapes found 10% of them diverging.

The fix was to stop repairing text: mapping construction happens in the
deserializer's visitor, which overwrites. No heuristics, no scope to get wrong.

The generalisable lesson is not "write more tests". It is that a heuristic that
approximates a spec needs the spec as the oracle, run against it, at the scale a
reader cannot check by eye.

## Follow-ups

- **The harness itself is not in the repository.** It was assembled ad hoc —
  Python plus `curl` plus fixture directories — which is why the bugs got caught
  and also why re-running it is a chore. The three high-value checks are cheap to
  make permanent:
  - a table-driven test pinning the analyzer against the reference regex,
    including the unconditional-continuation cases;
  - a frontmatter fixture corpus checked against PyYAML, covering the duplicate
    shapes;
  - the MCP smoke test already in `scripts/`, extended with the malformed-page
    cases the current fixture omits.
  Several of these landed as unit tests during the fix; the corpus-as-data part
  did not.
- One review claim — a hang under concurrent requests on a single MCP session —
  could not be reproduced, including 128 pooled keep-alive requests on one
  session. If real it is upstream in `rmcp`'s `StreamableHttpService`.

## References

- Related: [[2026-10-14-wi-mcp-server]], [[2026-10-14-wiki-search-ranking]],
  [[2026-10-15-wiki-tooling-and-agents-file]]
- Code: `scripts/mcp_smoke.py`, `scripts/smoke.sh`
- External: https://github.com/manti-by/odin/pull/41