# WI Tool Wiki — Open Questions

Questions raised by the Consistency Agent when wiki pages disagree and the discrepancy could not
be resolved from the codebase, connected MCPs, or other data sources. A human answers inline in
the **Answer** field; on its next run the Consistency Agent applies the answer to the affected
pages and moves the entry to **Resolved**.

## Open

_Newest first. Entry format:_

### Q-001 — Which filename is the session mapping: `.sessions.json` or `.openwiki-sessions.json`?

- **Date:** 2026-10-15
- **Pages:** [[2026-10-15-wiki-tooling-and-agents-file]]
- **Discrepancy:** `wiki/README.md` documents the mapping file as
  `.openwiki-sessions.json`, but the `wiki-sync` skill writes `wiki/.sessions.json`.
  The two disagree on the name, so a future run that trusts the README will look
  for a file the skill never writes, and vice versa.
- **Checked:** Read both the scaffolded `wiki/README.md` and the installed skill
  at `.opencode/skills/wiki-sync/SKILL.md`; they are not reconciled with each
  other anywhere in the repo, and `git log` has no commit introducing either.
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
