# WI Tool Wiki — Open Questions

Questions raised by the Consistency Agent when wiki pages disagree and the discrepancy could not
be resolved from the codebase, connected MCPs, or other data sources. A human answers inline in
the **Answer** field; on its next run the Consistency Agent applies the answer to the affected
pages and moves the entry to **Resolved**.

## Open

_Newest first. Entry format:_

### Q-002 — Should `Cargo.toml`'s `description` still say "OpenWiki MCP server"?

- **Date:** 2026-10-15
- **Pages:** [[2026-10-15-wiki-tooling-and-agents-file]]
- **Discrepancy:** The page records the stale description as a defect and then
  says "Aligned", but no alignment ever happened: `Cargo.toml` still reads
  `"OpenWiki MCP server: search and read Markdown wiki pages over MCP"`, and
  `git log -p --follow -- Cargo.toml` shows the line was introduced once in
  `2b58428` and never modified. The commit message for the session that
  identified the defect (`e9798ed`) also only *reports* it. Whether it is a
  defect is a product question: `AGENTS.md` calls the project "the **WI Tool**",
  while `sk.toml` calls it `wi-tool` with description "MCP server for wiki pages".
- **Checked:** `git log -p --follow -- Cargo.toml` and `-- sk.toml` for the
  edit history of the line; the `e9798ed` commit message, which lists the
  description as a finding but not as a fix; `AGENTS.md` and `sk.toml` for the
  current naming. Nothing in the repo says whether the crate is still OpenWiki's
  MCP server under a new name, or a separate product that inherited the
  description.
- **Answer:** _(human writes here)_

### Q-001 — Which filename is the session mapping: `.sessions.json` or `.openwiki-sessions.json`?

- **Date:** 2026-10-15
- **Pages:** [[2026-10-15-wiki-tooling-and-agents-file]]
- **Discrepancy:** `wiki/README.md` documents the mapping file as
  `.openwiki-sessions.json`, but the `wiki-sync` skill writes `wiki/.sessions.json`.
  The two disagree on the name, so a future run that trusts the README will look
  for a file the skill never writes, and vice versa.
- **Checked:** Read both the scaffolded `wiki/README.md` and the installed skill
  at `.opencode/skills/wiki-sync/SKILL.md`. Correction added 2026-10-15: the
  original entry said `git log` had no commit introducing either file, which is
  wrong — `e9798ed` added `wiki/.sessions.json` with one entry, and no
  `.openwiki-sessions.json` has ever been committed. So the *deployed* name is
  `.sessions.json`; what remains open is whether the README is the stale side.
- **Answer:** _(human writes here)_

<!--
### Q-00X — <short title of the discrepancy>

- **Date:** YYYY-MM-DD
- **Pages:** [[<page-a-filename-without-.md>]], [[<page-b-filename-without-.md>]]
- **Discrepancy:** <what the pages claim, and how the claims conflict>
- **Checked:** <sources consulted and why they didn't settle it — codebase paths, MCPs, docs>
- **Answer:** _(human writes here)_
-->

## Resolved

_Newest first. Moved here by the Consistency Agent, with a one-line note of what was applied._
