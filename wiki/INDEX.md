# WI Tool — Index

Session knowledge base for the OpenWiki project - one Markdown page per debugging
chase, investigation, code review, or set of changes. See [README.md](README.md) for conventions
and [TEMPLATE.md](TEMPLATE.md) for the page template. New pages are created or updated on demand
with `/wiki-sync`; the plugin only scaffolds the wiki.

## Pages

_Newest first._

- [Wiki tooling and AGENTS.md](pages/2026-10-15-wiki-tooling-and-agents-file.md) — Wiki + skill setup, stdio as the default transport, and the CI gate running neither tests nor clippy (2026-10-15)
- [wi — the OpenWiki MCP server](pages/2026-10-14-wi-mcp-server.md) — What the binary is, how it runs, why it exists, and what interface parity means in practice (2026-10-14)
- [Differential testing against the Python original](pages/2026-10-14-differential-testing-parity.md) — Why every porting bug was silent, and what caught them instead of unit tests (2026-10-14)
- [Wiki search ranking](pages/2026-10-14-wiki-search-ranking.md) — BM25F zone weights, the analyzer, and the corpus cache (2026-10-14)
- [Installer and release pipeline](pages/2026-10-14-installer-and-release-pipeline.md) — How a version bump becomes a release, and what the installer refuses to do (2026-10-14)

## By topic

_Topic clusters maintained by the Consistency Agent; topics with the most pages first.
`Open threads` is cross-cutting, not a cluster: those pages also appear above._

### Porting and parity

- [[2026-10-14-wi-mcp-server]] - the port itself and its interface contract
- [[2026-10-14-differential-testing-parity]] - the method that found the bugs
- [[2026-10-14-wiki-search-ranking]] - the ranking that had to match exactly

### Delivery

- [[2026-10-14-installer-and-release-pipeline]] - CI release and `curl | sh` install
- [[2026-10-15-wiki-tooling-and-agents-file]] - skills, wiki scaffolding, AGENTS.md,
  the stdio transport, and the `make check` / `make ci` split

### Open threads

- [[2026-10-15-wiki-tooling-and-agents-file]] - `serde_yaml` is unmaintained and the
  frontmatter parser depends on its behaviour; no successor has settled
- [[2026-10-15-wiki-tooling-and-agents-file]] - `Cargo.toml`'s `description` still says
  "OpenWiki MCP server" after the rename; nobody decided whether that is a defect (Q-002)
- [[2026-10-14-differential-testing-parity]] - the parity harness is not committed
- [[2026-10-14-wiki-search-ranking]] - mtime+size fingerprint has a theoretical blind spot