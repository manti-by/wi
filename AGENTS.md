# AGENTS.md

Guidance for coding agents working in this repository.

`wi` (the **WI Tool**) is a single Rust binary that serves a directory of
Markdown wiki pages over the Model Context Protocol. It is a port of the Python
wiki tools in [manti-by/odin#41](https://github.com/manti-by/odin/pull/41), and
**the reference implementation is the specification.** Tool names, argument
names, error strings and output text are expected to match it byte-for-byte.

## What this project is not

- Not a web server, a static site generator, or a wiki editor. It only reads.
- Not a general search engine. It ranks Markdown files in one directory.
- Not multi-tenant. One pages root per process, no auth, no persistence.

## Commands

```console
make help          # list every target
make ci            # check (fmt) + clippy --all-targets + test — the full gate
make check         # cargo fmt --check only
make clippy        # lint every target
make test          # unit tests only
make lint          # check + clippy
make smoke         # drive a real MCP session over BOTH transports
make run           # serve ./wiki/pages over HTTP (ARGS=... to override)
make release       # release binary
```

Run `make ci` before claiming anything works. Both workflows call it.

The naming is a trap worth stating once: **`make check` is formatting only.**
It was briefly mis-wired — CI invoked `make check` while the step was labelled
"cargo fmt, clippy and tests", so nothing in CI ran clippy or the 77 unit tests,
and `make lint` depended on a `fmt-check` target that no longer existed. Both are
fixed, but the lesson holds: when a workflow's step name and the target it
invokes disagree, one of them is lying, and it is usually the CI.

### Tests, and what they do not cover

Unit tests live beside the code as `#[cfg(test)]` modules. Two things they
deliberately do not do:

- **They do not test either transport.** `make smoke` does, by booting the
  binary and driving initialize / `tools/list` / `tools/call` — over stdio via
  `scripts/mcp_smoke_stdio.py`, over HTTP via `scripts/mcp_smoke.py`. A change
  to one transport must keep both passing; a client must not be able to tell
  which one it is on.
- **They do not test parity with the Python original.** A unit test written next
  to a reimplementation asserts the author's model of the original, not the
  original; when the model is wrong the test is wrong identically and passes.
  **When you touch parsing, the analyzer, scoring, or any output string, verify
  against the Python.** See "Verifying parity" below.

## Layout

- `src/main.rs` — CLI (clap), transport selection, bind, graceful shutdown.
  `--transport stdio` (default) or `--transport http`.
- `src/tools.rs` — the MCP surface: the three tool definitions, argument
  validation, output formatting, `ServerHandler` impl.
- `src/page.rs` — frontmatter parsing, metadata summary, snippets, and
  `resolve_page` (the path-containment check). The largest module, and the one
  carrying the most subtle requirements.
- `src/search.rs` — the analyzer and the BM25F scoring function.
- `src/corpus.rs` — the cached corpus snapshot, invalidation, and `rank`.
- `src/config.rs` — every tunable, with its reference counterpart named.
- `src/error.rs` — startup errors vs. tool errors, kept deliberately separate.

- `wiki/` — this project's own documentation. `README.md` is the schema,
  `TEMPLATE.md` the page template, `INDEX.md` the catalog, `QUESTIONS.md` the
  open-questions queue.
- `install.sh` — the `curl | sh` installer. Fails closed; see Security.
- `scripts/smoke.sh` + `scripts/mcp_smoke.py` — the MCP smoke test.
- `.github/workflows/` — `ci.yml` (every push and PR: `make check`, shell syntax,
  shell lint, `install.sh --help`, the installer smoke test, and a second job
  that builds with `--locked` and runs `make smoke`) and `release.yml`
  (check → version gate → four-way build → publish).
- `.github/scripts/` — release helpers: version gate, packaging, checksums,
  publish, shell lint, installer smoke test.

## Conventions

- Rust 2024 edition, one binary crate, no workspace and no `lib.rs`.
- **Comments explain *why*, not *what*.** A comment restating the code is noise;
  a comment explaining a non-obvious constraint or a discarded approach is the
  most valuable thing in the file. This codebase is dense with the latter on
  purpose — see `split_frontmatter` and the `MetaValue` deserializer.
- Every tunable in `src/config.rs` names the reference constant it mirrors.
  Changing a value there changes search behaviour for every user; do not tune
  one to "look better" without evidence.
- Errors are split in two on purpose. `ServeError` aborts startup. `ToolError`
  becomes an `is_error: true` result the agent reads. **A tool error is not a
  protocol error** — see the `call_tool` impl in `src/tools.rs`.
- Tests use real files in a temp directory, not mocks, for parsing and corpus
  behaviour. Each fixture gets a unique directory (see `write_temp`); keying on
  the file name alone made two tests race and failed one run in four.

## Things that will break silently

These are the failure modes that cost the most time on this project. All of them
produced no error and no failing test.

- **The analyzer feeds the corpus statistics.** Term tokenization determines
  field lengths, which determine the BM25 length-normalization divisor for every
  query. A change that looks cosmetic in `extract_terms` changes *which pages
  match*, not just their order.
- **`extract_terms` must implement `[a-z0-9][a-z0-9_.-]*` literally.** The
  continuation set is unconditional: once a term starts, `_`, `.` and `-` keep
  extending it whatever follows. `release.` and `self._private` are single terms.
  Requiring an alphanumeric after a separator is a plausible-looking bug that
  changes results.
- **`split_frontmatter` must implement `\A---\s*\n(.*?)\n---\s*\n?` including
  `\s*` backtracking.** Greedy-only gets `---\n---\nbody` right and
  `---\n\n---\nbody` wrong. `---foo` closes the block; an indented `  ---` does
  not; the opening `---` cannot also close it.
- **Repeated frontmatter keys resolve last-wins.** A strict YAML parser rejects
  them, and rejecting means the page vanishes from all three tools. This is
  handled in the `MetaValue` deserializer's `visit_map`, which overwrites. Do not
  reintroduce a text-level fix-up pass: two earlier attempts were wrong in ways
  that produced *wrong metadata* and were cubic in nesting depth.
- **Argument validation checks falsiness before type.** `{"query": 0}` is
  `Error: query is required`, not `must be a string`. A truthy non-string `name`
  fails with the generic `Wiki operation failed`, because the reference raised
  inside path resolution.
- **Display fallbacks are falsiness-based**, as in `meta.get(k) or '-'`. `title: 0`
  falls back to the file name — and the title feeds the top-weighted search zone.

## Security

- **stdout is the JSON-RPC channel in stdio mode.** Every log line goes to
  stderr; a single `println!` corrupts the stream and the client drops the
  session. `init_tracing` writes to stderr for exactly this reason — do not
  "simplify" it to stdout.
- **Never widen the bind without widening the `Host` allowlist.** The transport
  rejects unknown `Host` headers to stop DNS rebinding, and its default list is
  loopback-only. `--host 0.0.0.0` with the default list leaves the server
  reachable by a spoofed `Host: localhost` *and* unreachable by the address the
  operator asked for. `allowed_hosts()` in `src/main.rs` derives the list from
  `--host`; `--allowed-host` adds names. Preserve that coupling.
- **Nothing authenticates a client.** A non-loopback bind logs a warning for this
  reason. Do not "fix" the warning by removing it.
- `resolve_page` canonicalizes both the root and the target before the
  containment check, which is what makes `..`, absolute paths and symlinks safe.
  Keep both sides canonicalized.
- The installer fails closed on every path: a checksum it cannot verify, a
  corrupted archive, a missing `.sha256`, or a symlinked `wi` all abort with
  nothing written. Each case has a test in `.github/scripts/smoke-installer.sh`
  that asserts the install directory is left *completely empty* — "exited
  non-zero" is not the same guarantee as "wrote nothing".

## Verifying parity

This is the highest-value thing you can do here, and it is not yet a committed
script (see `wiki/pages/2026-10-14-differential-testing-parity.md`).

Fetch the reference:

```console
gh api "repos/manti-by/odin/contents/mcp_server/tools/wiki.py?ref=master" -q .content | base64 -d
gh api "repos/manti-by/odin/contents/mcp_server/tools/search.py?ref=master" -q .content | base64 -d
gh api "repos/manti-by/odin/contents/mcp_server/corpus.py?ref=master" -q .content | base64 -d
gh api "repos/manti-by/odin/contents/mcp_server/config.py?ref=master" -q .content | base64 -d
uv run --with pyyaml python3 -c 'import yaml; print(yaml.safe_load("a: 1"))'   # for frontmatter
```

Then, for whatever you changed: run the same inputs through both and diff. Use
thousands of generated inputs through one function at a time, not a handful of
examples through the whole binary. Compare *scores*, not just output shapes — a
divergence that only shows up in a score is exactly the kind that hides.

## Releasing

A version bump in `Cargo.toml` merged to `master` triggers the release. There is
no manual `gh release create`.

1. Bump `version` in `Cargo.toml`.
2. Bump `project.version` in `sk.toml` to match. `.github/scripts/detect-version.sh`
   fails the release if they disagree — this is inherited from `sk` so a release
   can never go out whose metadata contradicts its own tag.
3. Merge. CI builds the four platform binaries, writes checksums, publishes `v<version>`.

`workflow_dispatch` on the release workflow republishes without a version bump,
which is how a failed release is retried.

## Known issues

- **`serde_yaml` is deprecated and unmaintained** (0.9.34+deprecated; the upstream
  repository is archived). `serde_yml` is *also* deprecated — a shim forwarding to
  `noyalib` — so there is no drop-in successor. `src/page.rs` depends on this
  crate's behaviour more than its API suggests: the whole duplicate-key
  resolution is a `Deserialize` impl built on `deserialize_any` handing mapping
  entries to a visitor that overwrites. Any migration must preserve that, and
  the parity harness is what would prove it.
- The corpus fingerprint is mtime + size, so an edit preserving both would not
  invalidate the cache. Judged acceptable; a content hash would cost a full read
  per query, defeating the cache.
- Frontmatter nested deeper than 127 levels (measured, not estimated), or
  integers beyond 64 bits (`MetaValue::Int` is `i64`, `UInt` is `u64`), are
  outside what the YAML parser accepts. Such a page is skipped with a warning.
  The committed test only drives 2000 levels and asserts the skip is prompt
  (`src/page.rs:815`), so 127 is not itself pinned by a test.
- `PyYAML` implements YAML 1.1 and this binary's parser implements 1.2. A bare
  `yes`/`no`/`on`/`off` and leading-zero octal resolve differently. Documented in
  the README; quote those values.

## Wiki and skills

This repo documents itself. When you learn something non-obvious, write it down
rather than leaving it in the diff.

- One page per session under `wiki/pages/YYYY-MM-DD-<slug>.md`. Read
  `wiki/TEMPLATE.md` for the structure and `wiki/README.md` for the frontmatter
  schema and cross-link syntax.
- Cross-link with `[[filename-without-.md]]` and mirror it in `related:`.
- Add the page to `wiki/INDEX.md`, newest first.
- `.opencode/skills/` is generated by `sk sync` from the pins in `sk.toml` /
  `sk.lock`; it is gitignored. Edit `sk.toml` and re-run `sk sync` — never edit
  the installed skills directly.
- `AGENTS.md` should describe the repo as it *is*. When code and this file
  disagree, the code wins; fix this file in the same change.