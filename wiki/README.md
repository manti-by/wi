# wi — documentation wiki

These pages document `wi` itself. `wi` reads `pages/` from this directory by
default, so the project's own wiki is the best fixture to try it against:

```console
$ cargo run
2026-10-14T10:14:02Z  INFO wi: wi listening bound=127.0.0.1:4321 wiki_path=/repo/wiki/pages
```

Then point an MCP client at `http://127.0.0.1:4321/mcp` and ask it what this
project is and why the search ranking is weighted the way it is.

## Pages

| Page | About |
| --- | --- |
| [wi — OpenWiki MCP server](pages/2026-10-14-wi-mcp-server.md) | What the binary is, how it runs, and why it exists |
| [Wiki search ranking](pages/2026-10-14-wiki-search-ranking.md) | BM25F zone weights, the analyzer, and the corpus cache |
| [Installer and release pipeline](pages/2026-10-14-installer-and-release-pipeline.md) | How a version bump becomes a release, and what the installer refuses to do |

## Conventions

Pages are `YYYY-MM-DD-<kebab-topic>.md`, which makes the alphabetical listing
chronological and gives search a deterministic tie-break.

Frontmatter is optional but expected. `title`, `date`, `type` and `status` drive
the one-line summaries that `wiki_list_pages` prints; `tags` and `related` drive
search and cross-linking. `type` is one of `debug`, `investigation`,
`code-review` or `implementation`.

Write down *why* a decision was made and what was ruled out. A page that only
restates what the code does is already in the code, and goes stale.