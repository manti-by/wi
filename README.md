# wi

An MCP server for a Markdown wiki. One binary, no configuration file: point it
at a directory of `*.md` pages and it serves three tools that let an agent search
the wiki, read a page and browse the catalog.

```console
$ wi --wiki-path wiki/pages
2026-09-23T10:14:02Z  INFO wi: wi listening bound=127.0.0.1:4321 wiki_path=/repo/wiki/pages
```

## Install

```console
$ curl -LsSf https://raw.githubusercontent.com/manti-by/wi/master/install.sh | sh
```

This downloads the newest release for your machine, verifies its SHA-256 and
installs `wi` into `~/.local/bin`. macOS (Intel and Apple silicon) and
Ubuntu/Linux (x86-64 and arm64) are supported.

```console
$ curl -LsSf https://raw.githubusercontent.com/manti-by/wi/master/install.sh | sh -s -- --version v0.1.0
$ curl -LsSf https://raw.githubusercontent.com/manti-by/wi/master/install.sh | sh -s -- --dir /usr/local/bin
```

Uninstall is `rm ~/.local/bin/wi`. To build from source instead, use
`cargo install --path .` or `make install`.

## Usage

```console
$ wi --help
Serve a Markdown wiki over MCP

USAGE:
    wi [OPTIONS]

OPTIONS:
        --host <HOST>        Interface to bind [default: 127.0.0.1]
        --port <PORT>        Port to bind [default: 4321]
        --wiki-path <PATH>   Directory holding the `*.md` wiki pages [default: wiki/pages]
        --allowed-host <HOST>  Extra hostname or host:port to accept in the Host header
    -h, --help               Print help
    -V, --version            Print version
```

The server binds loopback by default, because it serves a project's private
notes. Nothing here authenticates a client, so exposing it is a deliberate act
and the server says so in a warning when it does:

```console
$ wi --host 0.0.0.0 --port 8080 --wiki-path ~/notes/wiki
WARN wi: serving on a non-loopback interface with no authentication; add
         --allowed-host for each name clients use to reach this address
```

The transport rejects requests whose `Host` header is not on its allowlist,
which is what stops a web page from reaching a loopback server through DNS
rebinding. Loopback names are always allowed; a specific `--host` adds itself.
When binding a wildcard there is no name clients would send, so add the names
they actually use:

```console
$ wi --host 0.0.0.0 --allowed-host wiki.internal --allowed-host 192.168.1.5
```

`--wiki-path` may be relative, in which case it resolves against the working
directory. The directory must exist when the server starts; pages added or
edited later are picked up automatically, because the corpus is re-fingerprinted
on every query.

## Connecting an MCP client

The server speaks Streamable HTTP. In `opencode.json`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "wi": {
      "type": "remote",
      "url": "http://127.0.0.1:4321/mcp",
      "enabled": true
    }
  }
}
```

In Claude Desktop's `claude_desktop_config.json`, run the binary as a local
server instead:

```json
{
  "mcpServers": {
    "wi": {
      "command": "wi",
      "args": ["--wiki-path", "/absolute/path/to/wiki/pages"]
    }
  }
}
```

## Tools

| Tool | Arguments | Returns |
| --- | --- | --- |
| `wiki_search` | `query` (required), `limit` | Ranked page names with scores and body snippets |
| `wiki_get_page` | `name` (required) | The full Markdown of one page, frontmatter included |
| `wiki_list_pages` | — | Every page with its title and metadata summary, no bodies |

### `wiki_search`

```console
$ wi --wiki-path wiki/pages &   # then call wiki_search
1. 2026-09-23-relay-mode.md (score 1.800)
   Relay mode computed on read — type: implementation, date: 2026-09-23, status: resolved, tags: relay, mode
   > L1: # Relay mode computed on read
   > L4: The relay mode is computed on read, not stored.

2. 2026-07-16-fix-type-errors.md (score 0.228)
   Fix type errors and verify CI flow — type: debug, date: 2026-07-16, status: resolved, services: ci
   > L3: The relay consumer had type errors in CI.
```

`limit` defaults to 5 and is clamped to 20. Queries longer than 500 characters
are rejected. A query that matches nothing — including one made only of stop
words — returns a short hint rather than an error, because "nothing found" is an
answer, not a failure.

## How search works

Pages are ranked with BM25F over three zones, each with its own boost:

| Zone | Source | Boost | Length normalization |
| --- | --- | --- | --- |
| Title | frontmatter `title` | 10.0 | off |
| Metadata | `tags`, `services`, `areas`, `tickets`, `type` | 5.0 | off |
| Body | the Markdown after the frontmatter | 1.0 | 0.75 |

Because each field is normalized against its own average length and the
saturation is applied once at the end, a long page cannot win on bulk alone and
a term repeated many times in one document cannot dominate the ranking on its
own. Equal scores break alphabetically by file name, which is also chronological,
so results are deterministic.

Text is lowercased and split on runs of `[a-z0-9]` followed by `[a-z0-9_.-]*`,
which keeps `relay.get_target_state` and `mnt-238` as single tokens. Tokens
shorter than two characters and a list of English stop words are dropped. The
same analyzer runs over documents and queries, so `relay` never matches inside
`relayed`.

The parsed corpus is cached and invalidated by comparing the name, modification
time and size of every `*.md` file, so an unchanged wiki is read from disk once
no matter how many searches run.

## Page format

A page is any `*.md` file directly inside the pages directory, with optional
YAML frontmatter:

```markdown
---
title: Relay mode computed on read
date: '2026-09-23'
type: implementation
status: resolved
services: [consumer, redis_bus]
tags: [relay, mode]
---
# Relay mode computed on read

The relay mode is computed on read, not stored.
```

Frontmatter is optional. `title`, `type`, `date` and `status` are for display;
`tags`, `services`, `areas`, `tickets` and `type` are also searched. A bare `-`
is read as the literal string rather than as YAML null, so `branch: -` means "no
branch". A page whose frontmatter is invalid YAML, or is not a mapping, is
skipped with a warning rather than served half-parsed. A repeated key keeps its
last value, the way YAML normally resolves it, rather than invalidating the page.
Carriage returns are normalized on read, so a page edited on Windows gets the
same line numbers as one edited on Linux.

`wiki_get_page` resolves a name against the pages root: the `.md` suffix is
optional, a leading `pages/` is stripped, and anything that resolves outside the
root — through `..`, an absolute path, or a symlink — is refused.

### Known divergence from the Python original

The Python implementation resolves frontmatter with PyYAML, which implements
YAML 1.1; this binary uses a 1.2 parser. A bare `yes`, `no`, `on` or `off` used
as a whole value therefore reads as the string here and as `True`/`False` there —
`status: yes` displays `status: yes` instead of `status: True`. Quoting the value
(`status: "yes"`) makes both agree. Everything else in the page format matches.

## Development

```console
$ make help          # list the targets
$ make check         # fmt --check, clippy and tests — what CI runs
$ make smoke         # start the binary and drive a real MCP session against it
$ make release       # build the release binary
```

`make smoke` is worth knowing about: the unit tests cover parsing and ranking,
but nothing in `cargo test` can tell you the transport starts and answers, so it
boots the binary, drives an initialize/list/call sequence over HTTP and asserts
on the text each tool returns.

### Releasing

Releases are cut by CI from a version bump. Bump `version` in `Cargo.toml`,
merge to `master`, and `.github/workflows/release.yml` builds the four platform
binaries and publishes `v<version>` with per-asset checksums. The publish job is
gated behind `make check`, so a red tree never releases. A manual
`workflow_dispatch` re-runs the publish without a version change, which is how a
failed release is retried.

## License

BSD-3-Clause.