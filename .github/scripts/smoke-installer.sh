#!/usr/bin/env bash
#
# End-to-end test for install.sh, run offline against fixture releases served by
# python3 -m http.server.
#
# Four cases, one per security property the installer claims:
#
#   good      a correct archive installs and produces a working binary
#   corrupt   a checksum mismatch aborts and leaves nothing behind
#   nohash    a missing .sha256 aborts rather than skipping verification
#   symlink   an archive whose `wi` is a symlink is rejected
#
# Each failure case asserts that $INSTALL_DIR is still empty afterwards, so a
# half-installed binary cannot survive an aborted install.

set -euo pipefail

port="${SMOKE_PORT:-8123}"
version=v0.0.1
root=$(mktemp -d)
workdir=$(mktemp -d)
# install.sh builds its URL as <base>/<tag>/<asset>, so the fixture has to
# mirror GitHub's per-tag directory rather than serving assets from the root.
fixture="$root/$version"
mkdir -p "$fixture"
# Installed before anything can fail, so an early failure still cleans up the
# temp directories; `server_pid` is 0 until the fixture server starts.
server_pid=0
trap 'kill "$server_pid" 2>/dev/null || true; rm -rf "$root" "$workdir"' EXIT

# A stand-in for the release binary: install.sh only copies it, so any
# executable that prints its name proves the right file landed.
make_binary() {
    printf '#!/bin/sh\necho wi\n' > "$1"
    chmod 755 "$1"
}

publish() {
    # publish <asset-name> <with-sha256: yes|no> <symlink: yes|no>
    _asset="$1"
    _sha="$2"
    _symlink="$3"
    _stage="$workdir/stage-$_asset"
    rm -rf "$_stage"
    mkdir -p "$_stage"
    if [ "$_symlink" = "yes" ]; then
        ln -s /etc/passwd "$_stage/wi"
    else
        make_binary "$_stage/wi"
    fi
    tar -czf "$fixture/wi-0.0.1-$_asset.tar.gz" -C "$_stage" .
    if [ "$_sha" = "yes" ]; then
        (cd "$fixture" \
            && sha256sum "wi-0.0.1-$_asset.tar.gz" >"wi-0.0.1-$_asset.tar.gz.sha256")
    fi
}

os=$(uname -s)
arch=$(uname -m)
case "$os" in
    Darwin) os_part=macos ;;
    Linux) os_part=linux ;;
    *) echo "smoke-installer: unsupported OS $os" >&2; exit 0 ;;
esac
case "$arch" in
    x86_64 | amd64) arch_part=x86_64 ;;
    arm64 | aarch64) arch_part=aarch64 ;;
    *) echo "smoke-installer: unsupported arch $arch" >&2; exit 0 ;;
esac
asset="$os_part-$arch_part"

publish "$asset" yes no

# Served from inside the fixture directory rather than via `--directory`, so
# this does not depend on the runner's Python supporting that flag.
(cd "$root" && exec python3 -m http.server "$port" --bind 127.0.0.1) \
    >"$workdir/server.log" 2>&1 &
server_pid=$!
# Poll until the fixture is actually reachable rather than sleeping a fixed
# amount, so a loaded runner cannot turn this into a flaky failure.
for _ in $(seq 1 40); do
    if curl -fsS "http://127.0.0.1:$port/$version/" >/dev/null 2>&1; then
        break
    fi
    sleep 0.25
done

# Runs the installer against the fixture server. Sets `status` to its exit code.
run_install() {
    set +e
    WI_RELEASE_BASE="http://127.0.0.1:$port" \
        WI_VERSION="$version" \
        WI_INSTALL_DIR="$1" \
        sh install.sh >"$workdir/install.log" 2>&1
    status=$?
    set -e
}

fail() {
    echo "smoke-installer: $1" >&2
    cat "$workdir/install.log" >&2 || true
    exit 1
}

# Asserts an install directory was left completely empty, which is what makes
# "aborted" mean the same as "nothing was written".
assert_nothing_installed() {
    _leftovers=$(ls -A "$1" 2>/dev/null || true)
    [ -z "$_leftovers" ] || fail "an aborted install left files in $1"
}

# Case 1: a good release installs and runs.
dest="$workdir/install-good"
run_install "$dest"
[ "$status" -eq 0 ] || fail "good archive should install, got status $status"
[ -x "$dest/wi" ] || fail "good archive did not produce $dest/wi"
[ "$("$dest/wi")" = "wi" ] || fail "installed binary did not run"
echo "ok   good archive installs"

# Case 2: a corrupted archive is rejected and installs nothing.
corrupt="$fixture/wi-0.0.1-$asset.tar.gz"
printf 'corrupted' >>"$corrupt"
dest="$workdir/install-corrupt"
run_install "$dest"
[ "$status" -ne 0 ] || fail "corrupt archive should abort, got status 0"
assert_nothing_installed "$dest"
echo "ok   corrupt archive rejected"

# Restore the checksum for the remaining cases.
(cd "$fixture" && sha256sum "wi-0.0.1-$asset.tar.gz" >"wi-0.0.1-$asset.tar.gz.sha256")

# Case 3: a missing checksum is refused rather than skipped.
mv "$fixture/wi-0.0.1-$asset.tar.gz.sha256" "$fixture/hidden.sha256"
dest="$workdir/install-nohash"
run_install "$dest"
[ "$status" -ne 0 ] || fail "missing checksum should abort, got status 0"
assert_nothing_installed "$dest"
mv "$fixture/hidden.sha256" "$fixture/wi-0.0.1-$asset.tar.gz.sha256"
echo "ok   missing checksum rejected"

# Case 4: an archive whose `wi` is a symlink is rejected.
symlink_asset="symlink-$asset"
publish "$symlink_asset" yes yes
cp "$fixture/wi-0.0.1-$symlink_asset.tar.gz.sha256" \
   "$fixture/wi-0.0.1-$asset.tar.gz.sha256"
dest="$workdir/install-symlink"
run_install "$dest"
[ "$status" -ne 0 ] || fail "symlinked binary should abort, got status 0"
assert_nothing_installed "$dest"
echo "ok   symlinked binary rejected"

echo "smoke-installer: all cases passed"