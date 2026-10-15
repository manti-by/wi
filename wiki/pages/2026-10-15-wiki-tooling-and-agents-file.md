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
after the project was renamed to *WI Tool*. Aligned.

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