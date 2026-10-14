#!/usr/bin/env bash
#
# Publishes the release, or repairs one that already exists.
#
# A manual re-run has to be able to fix a release whose creation succeeded but
# whose asset upload failed, so an existing release is updated in place rather
# than treated as done. --clobber makes that re-upload idempotent.
#
# Environment:
#   VERSION   the version being released, without a leading v
#   SHA       the commit the tag should point at
#   GH_TOKEN  a token the gh CLI accepts
#
# Uploads dist/*. Must run from the repository root, after checksums.sh.

set -euo pipefail

tag="v$VERSION"

if gh release view "$tag" >/dev/null 2>&1; then
    echo "release $tag already exists; re-uploading assets over it"
    gh release upload "$tag" dist/* --clobber
else
    gh release create "$tag" \
        --target "$SHA" \
        --title "wi $tag" \
        --generate-notes \
        dist/*
fi