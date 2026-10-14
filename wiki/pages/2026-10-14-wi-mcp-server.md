---
title: 'wi — OpenWiki MCP server'
date: '2026-10-14'
type: implementation
status: resolved
tickets: [MNT-238]
tags: [mcp, wiki, search, bm25, rust, release]
related: [2026-10-14-wiki-search-ranking, 2026-10-14-installer-and-release-pipeline]
---
# wi — OpenWiki MCP server

## TL;DR

`wi` is a single Rust binary that serves a directory of Markdown wiki pages over
MCP. It exposes three tools — `wiki_search`, `wiki_get_page` and
`wiki_list_pages` — and replaces a Python MCP server that did the same thing
inside another project.

## Running it

```console
$ wi --wiki-path wiki/pages
```

Defaults: loopback, port 4321, pages read from `wiki/pages/`. All three are
overridable with `--host`, `--port` and `--wiki-path`.

## Why a standalone binary

The wiki tools originally lived inside a larger Python MCP server, which meant
every project wanting wiki search had to vendor that whole server and its
dependencies. The search itself is self-contained — parse frontmatter, tokenize,
rank — so it was extracted into a binary that installs with `curl | sh` the same
way [`sk`](https://github.com/manti-by/sk) does.

Extracting it also made the tool portable to projects written in anything:
`wi` does not care what language a repository is in, only where its `wiki/pages/`
directory is.

## Interface parity

The tool names, argument names, error strings and output formatting are
byte-for-byte what the Python implementation produced, so an agent that was
prompted against one behaves the same against the other. Two things are
deliberately *not* changed: results come back as plain text rather than
structured content, and a "nothing matched" answer is a success rather than an
error.

Parity is stated here as a contract rather than an aspiration, and it is enforced
by tests that pin behaviour the code is not obviously right about:

- the argument checks test falsiness *before* type, so `{"query": 0}` is a
  missing query rather than a mistyped one, and a truthy non-string `name` fails
  generically because the reference raised inside path resolution;
- a repeated frontmatter key keeps its last value instead of invalidating the
  page, which a strict YAML parser would do;
- the frontmatter delimiter is read as a regex, not as "a line containing
  exactly `---`", which decides whether `---\n---\nbody` has frontmatter at all;
- CRLF is normalized on read, because it shifts every body offset and every
  snippet line number.

One divergence is accepted and documented in the README: PyYAML implements YAML
1.1 and reads a bare `yes`/`no`/`on`/`off` as a boolean. This binary uses a 1.2
parser and reads them as strings. Emulating 1.1 would mean reimplementing
scalar resolution to emulate a quirk most writers consider a bug.

See [wiki search ranking](2026-10-14-wiki-search-ranking.md) for why the ranking
behaves the way it does, and
[installer and release pipeline](2026-10-14-installer-and-release-pipeline.md)
for how a version bump becomes a release.