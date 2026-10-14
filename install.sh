#!/bin/sh
#
# wi installer for macOS and Ubuntu/Linux.
#
#   curl -LsSf https://raw.githubusercontent.com/manti-by/wi/master/install.sh | sh
#
# Downloads the newest release binary for this machine, verifies its SHA-256
# and installs it into ~/.local/bin (or $WI_INSTALL_DIR). Written in POSIX sh
# because macOS ships bash 3.2 and Ubuntu ships dash; it deliberately avoids
# bashisms and optional tools.

set -eu

REPO="${WI_REPO:-manti-by/wi}"
# Resolved after the flags are parsed, so `--dir` wins even without a $HOME.
INSTALL_DIR="${WI_INSTALL_DIR:-}"
API="https://api.github.com/repos/$REPO/releases/latest"
RELEASE_BASE="${WI_RELEASE_BASE:-https://github.com/$REPO/releases/download}"
VERSION="${WI_VERSION:-}"

usage() {
    cat <<'EOF'
Install the latest wi release from GitHub.

USAGE:
    install.sh [OPTIONS]

OPTIONS:
    --version <tag>   Install a specific release tag (for example v0.1.0)
    --dir <path>      Install the binary into <path> (default: ~/.local/bin)
    -h, --help        Print this help and exit

ENVIRONMENT:
    WI_REPO           owner/repo to pull releases from (default: manti-by/wi)
    WI_VERSION        the same as --version
    WI_INSTALL_DIR    the same as --dir
    WI_RELEASE_BASE   base URL that holds the release assets
    GITHUB_TOKEN      token used for the GitHub API, raising its rate limit;
                      GH_TOKEN is accepted as an alternative
EOF
}

error() {
    echo "error: $1" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            [ $# -ge 2 ] || error "missing value for --version"
            VERSION="$2"
            shift 2
            ;;
        --dir)
            [ $# -ge 2 ] || error "missing value for --dir"
            INSTALL_DIR="$2"
            shift 2
            ;;
        -h | --help)
            usage
            exit 0
            ;;
        *)
            error "unknown option \`$1\`; run with --help"
            ;;
    esac
done

if [ -z "$INSTALL_DIR" ]; then
    [ -n "${HOME:-}" ] || error "HOME is not set; pass --dir <path> or set WI_INSTALL_DIR"
    INSTALL_DIR="$HOME/.local/bin"
fi

# Fetches a URL to stdout, using curl or wget and the GitHub token when one is
# set. `-f`/`--fail` turns an HTTP error into a non-zero exit.
http_get() {
    _url="$1"
    _token="${GITHUB_TOKEN:-${GH_TOKEN:-}}"
    if command -v curl >/dev/null 2>&1; then
        if [ -n "$_token" ]; then
            curl -fsSL -H "Authorization: Bearer $_token" "$_url"
        else
            curl -fsSL "$_url"
        fi
    elif command -v wget >/dev/null 2>&1; then
        if [ -n "$_token" ]; then
            wget -qO- --header="Authorization: Bearer $_token" "$_url"
        else
            wget -qO- "$_url"
        fi
    else
        error "curl or wget is required to download wi"
    fi
}

# The tag of the newest published release, read out of the API JSON with sed so
# there is no jq dependency.
latest_tag() {
    _body=$(http_get "$API") || error "could not reach the GitHub API at $API"
    _tag=$(printf '%s\n' "$_body" |
        sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' |
        head -n 1)
    [ -n "$_tag" ] || error "no published release was found for $REPO"
    printf '%s\n' "$_tag"
}

# Verifies an archive against the sibling .sha256 file, whichever hash tool the
# platform provides. Every branch fails closed: an archive that cannot be
# verified is never installed.
verify() {
    _file="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c "$_file.sha256" || error "checksum mismatch for $_file"
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c "$_file.sha256" || error "checksum mismatch for $_file"
    elif command -v openssl >/dev/null 2>&1; then
        _expected=$(cut -d' ' -f1 <"$_file.sha256")
        _actual=$(openssl dgst -sha256 "$_file" | sed 's/.*= *//')
        [ "$_expected" = "$_actual" ] || error "checksum mismatch for $_file"
    else
        error "no sha256 tool found (sha256sum, shasum or openssl); cannot verify $_file"
    fi
}

os=$(uname -s)
arch=$(uname -m)

case "$os" in
    Darwin) os_part=macos ;;
    Linux) os_part=linux ;;
    *) error "unsupported operating system \`$os\`; wi ships binaries for macOS and Linux" ;;
esac

case "$arch" in
    x86_64 | amd64) arch_part=x86_64 ;;
    arm64 | aarch64) arch_part=aarch64 ;;
    *) error "unsupported architecture \`$arch\` on $os" ;;
esac

target="$os_part-$arch_part"

if [ -n "$VERSION" ]; then
    tag="$VERSION"
else
    tag=$(latest_tag)
fi
version="${tag#v}"

archive="wi-$version-$target.tar.gz"
base="$RELEASE_BASE/$tag"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT HUP INT TERM

echo "Downloading wi $version for $target"
http_get "$base/$archive" >"$tmp/$archive" ||
    error "could not download $base/$archive"

# Checksums are mandatory: an absent or unreachable .sha256 aborts rather than
# silently installing an unverified binary.
if ! http_get "$base/$archive.sha256" >"$tmp/$archive.sha256" 2>/dev/null; then
    error "no checksum published for $archive; refusing to install unverified"
fi
(cd "$tmp" && verify "$archive")

tar -xzf "$tmp/$archive" -C "$tmp" || error "could not extract $archive"
# Reject a symlinked binary: `[ -f ]` follows links, so an archive could smuggle
# `wi -> /etc/passwd` and have its target copied into $INSTALL_DIR. Written as an
# `if` rather than `[ … ] && [ … ] || error` so the two conditions read as a
# single check (shellcheck SC2015).
if [ ! -f "$tmp/wi" ] || [ -L "$tmp/wi" ]; then
    error "$archive did not contain a regular wi binary"
fi

mkdir -p "$INSTALL_DIR" || error "could not create $INSTALL_DIR"

staged="$INSTALL_DIR/.wi.tmp.$$"
if cp "$tmp/wi" "$staged" && chmod 755 "$staged"; then
    mv -f "$staged" "$INSTALL_DIR/wi"
else
    rm -f "$staged"
    error "could not write to $INSTALL_DIR; set WI_INSTALL_DIR to a writable directory"
fi

echo "Installed wi $version to $INSTALL_DIR/wi"

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "note: $INSTALL_DIR is not on your PATH; add it to run \`wi\`" ;;
esac