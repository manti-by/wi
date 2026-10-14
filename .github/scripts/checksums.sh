#!/usr/bin/env bash
#
# Writes the checksums that ship with a release.
#
# Two shapes, for two consumers: one aggregate checksums.txt for a human to
# eyeball, and a per-asset .sha256 that install.sh downloads and feeds to
# `sha256sum -c`. The per-asset files use a ./ prefix so the name inside the
# checksum matches what install.sh has in its temp directory.
#
# Reads dist/*.tar.gz, which the release job gets from downloading the build
# artifacts. Must run from the repository root.

set -euo pipefail

cd dist

shopt -s nullglob
archives=(./*.tar.gz)
if [ "${#archives[@]}" -eq 0 ]; then
    echo "no archives in dist/; the build artifacts did not download" >&2
    exit 1
fi

sha256sum "${archives[@]}" > checksums.txt

for f in "${archives[@]}"; do
    sha256sum "$f" > "$f.sha256"
done

cat checksums.txt