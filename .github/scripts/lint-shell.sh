#!/usr/bin/env bash
#
# Runs shellcheck over every shell file the repository owns.
#
# Skips silently when shellcheck is absent, so the gate still works on a
# machine or runner without it.

set -euo pipefail

if ! command -v shellcheck >/dev/null 2>&1; then
    echo "shellcheck not installed; skipping"
    exit 0
fi

# The installer's POSIX-sh portability matters more than its lint cleanliness,
# so -s (POSIX mode) is what matters here.
shellcheck -s sh install.sh
shellcheck .github/scripts/*.sh

# Newer shellcheck drops checks rather than adding them, so a clean run locally
# can still be red on a runner pinned to an older build. Report the version so a
# CI failure can be reproduced with the same one.
shellcheck --version | grep version: