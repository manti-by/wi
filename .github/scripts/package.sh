#!/usr/bin/env bash
#
# Stages one built binary into a release archive.
#
# The archive holds `wi` at its root plus whichever of README.md and the
# licence files exist in the tree. install.sh extracts to a temp directory and
# copies `wi` out, so the layout has to keep that name.
#
# Environment:
#   VERSION  the version being released, without a leading v
#   TARGET   the rust target triple that was built
#   ASSET    the asset suffix, for example linux-x86_64
#
# Writes dist/wi-<version>-<asset>.tar.gz. Must run from the repository root,
# after `cargo build --release --target $TARGET`.

set -euo pipefail

staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT

cp "target/$TARGET/release/wi" "$staging/wi"

for extra in README.md LICENSE LICENSE.md; do
    if [ -f "$extra" ]; then
        cp "$extra" "$staging/"
    fi
done

mkdir -p dist
tar -czf "dist/wi-$VERSION-$ASSET.tar.gz" -C "$staging" .

echo "Packaged dist/wi-$VERSION-$ASSET.tar.gz"