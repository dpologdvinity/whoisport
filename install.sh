#!/bin/sh
# whoisport installer for Linux and macOS.
#
#   curl -fsSL https://github.com/dpologdvinity/whoisport/releases/latest/download/install.sh | sh
#
# It finds the release binary for this machine, downloads it, checks its SHA-256 against
# the release's SHA256SUMS file, and installs it. Nothing is installed if the check fails.
# Windows has its own installer, install.ps1.
#
# Options (arguments after `sh -s --`, or environment variables):
#   --version <tag>   WHOISPORT_VERSION      release to install (default: the latest)
#   --dir <path>      WHOISPORT_INSTALL_DIR  where to put the binary (default: ~/.local/bin)
#   --help
#
# Requires: curl, tar, and sha256sum or shasum (all present on a stock Linux or macOS).
#
# The whole script is wrapped in functions and only runs on the last line, so a download
# that is cut off halfway can never execute a partial command.

set -eu

REPO="dpologdvinity/whoisport"
# Overridden by the tests to serve a fake release from disk (file:// URLs).
BASE_URL="${WHOISPORT_BASE_URL:-https://github.com/$REPO/releases}"

# Print a progress message.
say() {
    printf 'whoisport-install: %s\n' "$*"
}

# Print an error to stderr and stop.
fail() {
    printf 'whoisport-install: error: %s\n' "$*" >&2
    exit 1
}

# Stop unless the command `$1` is available.
need() {
    command -v "$1" >/dev/null 2>&1 || fail "'$1' is required but was not found"
}

# Print the release target for this machine, e.g. `aarch64-apple-darwin`.
# Linux builds are static (musl), so they run on any distribution.
detect_target() {
    detect_os=$(uname -s)
    detect_arch=$(uname -m)
    case "$detect_os" in
        Linux) detect_os_part="unknown-linux-musl" ;;
        Darwin) detect_os_part="apple-darwin" ;;
        *) fail "unsupported operating system: $detect_os (on Windows use install.ps1)" ;;
    esac
    case "$detect_arch" in
        x86_64 | amd64) detect_arch_part="x86_64" ;;
        aarch64 | arm64) detect_arch_part="aarch64" ;;
        *) fail "unsupported CPU architecture: $detect_arch" ;;
    esac
    # A shell running under Rosetta on an Apple silicon Mac reports x86_64; the native
    # binary is the better choice there.
    if [ "$detect_os" = "Darwin" ] && [ "$detect_arch_part" = "x86_64" ] &&
        [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = "1" ]; then
        detect_arch_part="aarch64"
    fi
    printf '%s-%s' "$detect_arch_part" "$detect_os_part"
}

# Print the release tag to install: $1 if given (with or without a leading "v"), else the
# latest release. The latest is found from the redirect of /releases/latest, which needs
# no API call (and so has no rate limit).
resolve_version() {
    if [ -n "$1" ]; then
        case "$1" in
            v*) printf '%s' "$1" ;;
            *) printf 'v%s' "$1" ;;
        esac
        return
    fi
    resolve_url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$BASE_URL/latest") ||
        fail "could not look up the latest release"
    case "$resolve_url" in
        */tag/v*) printf '%s' "${resolve_url##*/tag/}" ;;
        *) fail "could not find the latest release version (got: $resolve_url)" ;;
    esac
}

# Print the SHA-256 of file $1.
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    else
        shasum -a 256 "$1" | cut -d ' ' -f 1
    fi
}

# Stop unless file $1 matches the hash listed for it in the SHA256SUMS file $2.
# SHA256SUMS lines look like `<hash>  <file name>` (or `<hash> *<file name>`).
verify_checksum() {
    verify_name=$(basename "$1")
    verify_expected=$(awk -v name="$verify_name" '$2 == name || $2 == "*" name { print $1 }' "$2")
    [ -n "$verify_expected" ] || fail "no checksum for $verify_name in SHA256SUMS; refusing to install an unverified file"
    verify_actual=$(sha256_of "$1")
    [ "$verify_actual" = "$verify_expected" ] ||
        fail "checksum mismatch for $verify_name (expected $verify_expected, got $verify_actual); nothing was installed"
}

# Print how to use the installer. (Self-contained: when piped from curl there is no file
# to read it from.)
usage() {
    cat <<'USAGE'
whoisport installer for Linux and macOS

  curl -fsSL https://github.com/dpologdvinity/whoisport/releases/latest/download/install.sh | sh
  curl -fsSL https://github.com/dpologdvinity/whoisport/releases/latest/download/install.sh | sh -s -- --version v0.5.7

Options (or the environment variables beside them):
  --version <tag>   WHOISPORT_VERSION      release to install (default: the latest)
  --dir <path>      WHOISPORT_INSTALL_DIR  where to put the binary (default: ~/.local/bin)
  --help

The binary's SHA-256 is checked against the release's SHA256SUMS before anything is installed.
USAGE
}

# Install whoisport. Arguments are the command-line options (see the header).
main() {
    main_version="${WHOISPORT_VERSION:-}"
    main_dir="${WHOISPORT_INSTALL_DIR:-$HOME/.local/bin}"
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --version) [ "$#" -ge 2 ] || fail "--version needs a value"; main_version="$2"; shift 2 ;;
            --dir) [ "$#" -ge 2 ] || fail "--dir needs a value"; main_dir="$2"; shift 2 ;;
            -h | --help) usage; return 0 ;;
            *) fail "unknown option: $1 (try --help)" ;;
        esac
    done

    need curl
    need tar
    command -v sha256sum >/dev/null 2>&1 || command -v shasum >/dev/null 2>&1 ||
        fail "'sha256sum' or 'shasum' is required but neither was found"

    main_target=$(detect_target)
    main_tag=$(resolve_version "$main_version")
    main_file="whoisport-$main_tag-$main_target.tar.gz"
    main_url="$BASE_URL/download/$main_tag"
    say "installing whoisport $main_tag for $main_target"

    # A private temporary directory, always removed on exit.
    main_tmp=$(mktemp -d)
    trap 'rm -rf "$main_tmp"' EXIT INT TERM

    curl -fsSL --retry 3 -o "$main_tmp/$main_file" "$main_url/$main_file" ||
        fail "could not download $main_url/$main_file (does release $main_tag exist for $main_target?)"
    curl -fsSL --retry 3 -o "$main_tmp/SHA256SUMS" "$main_url/SHA256SUMS" ||
        fail "could not download $main_url/SHA256SUMS; refusing to install an unverified file"
    verify_checksum "$main_tmp/$main_file" "$main_tmp/SHA256SUMS"
    say "checksum OK"

    tar -xzf "$main_tmp/$main_file" -C "$main_tmp"
    [ -f "$main_tmp/whoisport" ] || fail "the archive did not contain a whoisport binary"

    mkdir -p "$main_dir"
    # Copy beside the destination, then rename: replacing a running whoisport is atomic.
    cp "$main_tmp/whoisport" "$main_dir/.whoisport.new"
    chmod 755 "$main_dir/.whoisport.new"
    mv -f "$main_dir/.whoisport.new" "$main_dir/whoisport"

    say "installed $("$main_dir/whoisport" --version) to $main_dir/whoisport"
    case ":$PATH:" in
        *":$main_dir:"*) ;;
        *) say "note: $main_dir is not on your PATH; add it, e.g.  export PATH=\"$main_dir:\$PATH\"" ;;
    esac
}

# Run, unless a test has sourced this file to call the functions above.
if [ -z "${WHOISPORT_INSTALL_SOURCED:-}" ]; then
    main "$@"
fi
