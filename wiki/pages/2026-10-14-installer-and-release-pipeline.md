---
title: 'Installer and release pipeline'
date: '2026-10-14'
type: implementation
status: resolved
tickets: [MNT-238]
tags: [ci, release, installer, security]
related: [2026-10-14-wi-mcp-server]
---
# Installer and release pipeline

## TL;DR

A version bump in `Cargo.toml`, merged to `master`, produces four platform
binaries and a GitHub release with per-asset checksums. `install.sh` fetches,
verifies and installs one of them into `~/.local/bin`.

## The release path

`.github/workflows/release.yml` has four jobs:

1. **check** — `make check` (`cargo fmt --check`, `clippy --all-targets`,
   `cargo test`). The publish job depends on it, so a red tree never releases.
2. **version** — reads `version` out of `Cargo.toml` and compares it against the
   same line at the pre-push commit. A `workflow_dispatch` always counts as
   changed; that is how a failed release is retried.
3. **build** — a four-way matrix of native runners, each producing
   `dist/wi-<version>-<asset>.tar.gz`. No cross-compilation and no musl: each
   runner builds for its own platform.
4. **release** — downloads the artifacts, writes checksums, publishes.

The `build` job is gated on `changed == 'true'`, so ordinary pushes to `master`
produce a green, no-op release run rather than a skipped-then-failed one.

Concurrency is `release-${{ github.ref }}` with `cancel-in-progress: false`.
Cancelling here would let a second push tear down an in-flight release halfway
through uploading assets, leaving a release page with missing files.

## Why a diff-based version gate

The alternative — "publish whenever the tag does not exist" — publishes on any
push that happens to precede a tag, and does nothing useful otherwise. Comparing
the version against its predecessor is a statement about intent: the version is
the release trigger, and CI is the only thing that acts on it. A human never runs
`gh release create` by hand.

## Checksums are mandatory

Every asset ships a sibling `.sha256`, and `install.sh` refuses to install an
archive whose checksum it cannot fetch or verify. There is no "skip verification
if unavailable" path: an unverifiable binary is not installed, period. That
tradeoff is deliberate — a user on a flaky network re-runs the installer, and a
user who cannot verify should not be running the binary at all.

The `.sha256` files use a `./` prefix so the filename inside the checksum matches
what `install.sh` has staged in its temp directory, which is what makes a bare
`sha256sum -c` work without argument juggling.

## What the installer refuses to do

Four things, each covered by a test in `.github/scripts/smoke-installer.sh`:

- **A corrupt archive.** The checksum comparison fails and the install aborts.
- **A missing checksum.** Skipping verification is not an option.
- **A symlinked binary.** An archive could contain `wi -> /etc/passwd`, and a
  naive `[ -f ]` test follows symlinks — so the copy would faithfully install
  `/etc/passwd` as `wi`. The check is `[ ! -f ... ] || [ -L ... ]`.
- **A partial write.** The binary is copied to a temp name in the destination
  directory and `mv`-ed into place, so an interrupted install cannot leave a
  truncated `wi` on `PATH`.

Each failing case asserts the install directory is *completely empty*
afterwards. "The installer exited non-zero" is not the same guarantee as "the
installer wrote nothing", and only the second one is worth anything.

## Why the smoke test serves fixtures over HTTP

The installer only ever talks to a URL, so testing it against the real GitHub API
would make the test slow, flaky and dependent on the network. The smoke test
serves generated fixture releases from `python3 -m http.server` on loopback and
points the installer at them with `WI_RELEASE_BASE`. It also mirrors GitHub's
per-tag directory layout, because that is part of the URL contract — an earlier
version served assets from the root and every case 404'd, which is exactly the
kind of mismatch the test exists to catch.

## What is deliberately not here

No Homebrew tap, no Nix, no apt/deb packages, no `cargo publish`. The install
script covers `curl | sh` the way `uv` does, which is the whole documented path.
Adding a second distribution channel means maintaining it; if nobody has asked,
it is not worth the release matrix it would double.