#!/usr/bin/env bash
#
# Decides whether a push to master warrants a release, and reports the version
# it would publish.
#
# Cargo.toml is the source of truth for the version. The comparison is against
# the `version` line at $BEFORE, the commit the pushed ref pointed at
# beforehand. An all-zero BEFORE means a brand-new branch with no predecessor,
# which counts as a change.
#
# Environment:
#   BEFORE         the commit the ref pointed at before this push
#   EVENT_NAME     github.event_name; a manual dispatch always counts as changed
#   GITHUB_OUTPUT  file to receive the `version` and `changed` outputs
#
# Must run from the repository root, with the full history checked out.

set -euo pipefail

# Prints the first `version = "…"` line of the manifest at $1. Only lines
# starting the line count, so an inline dependency version never matches.
read_version() {
    awk -F'"' '/^version[[:space:]]*=/ { print $2; exit }' "$1"
}

cargo_version=$(read_version Cargo.toml)

if [ -z "$cargo_version" ]; then
    echo "could not read version out of Cargo.toml" >&2
    exit 1
fi

previous=''
all_zero=0000000000000000000000000000000000000000
if [ -n "${BEFORE:-}" ] && [ "$BEFORE" != "$all_zero" ]; then
    # A missing or unreadable before-commit is treated as "no previous
    # version", which reads as a change.
    previous=$(git show "$BEFORE:Cargo.toml" 2>/dev/null \
        | awk -F'"' '/^version[[:space:]]*=/ { print $2; exit }' || true)
fi

if [ "${EVENT_NAME:-}" = "workflow_dispatch" ] || [ "$cargo_version" != "$previous" ]; then
    changed=true
else
    changed=false
fi

echo "Detected version $cargo_version (was '${previous:-none}'), changed=$changed"
echo "version=$cargo_version" >> "$GITHUB_OUTPUT"
echo "changed=$changed" >> "$GITHUB_OUTPUT"