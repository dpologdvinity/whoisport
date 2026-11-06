#!/usr/bin/env bash
# Tests for apt/build.sh. Run with:  bash tests/apt_test.sh
#
# Needs dpkg-deb, apt-ftparchive (apt-utils), apt-get and gpg; no root and no network.
# Two fake .deb packages (amd64 and arm64) are signed into a repository with a throwaway key,
# then the real apt-get reads it through a file: URI, in a private state directory so the
# system's apt configuration is never touched.
# Covers: apt accepts the signature, every architecture lists the package, a download is
# checksum-verified, and a tampered .deb or a repository signed by another key is refused.

set -euo pipefail

here=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
failures=0

# Report one test result.
check() {
    if [ "$2" = "ok" ]; then
        printf 'ok   %s\n' "$1"
    else
        printf 'FAIL %s: %s\n' "$1" "$2"
        failures=$((failures + 1))
    fi
}

# Make a throwaway signing key in its own keyring; prints the fingerprint.
new_key() {
    GNUPGHOME=$1 gpg --batch --quiet --passphrase '' --quick-gen-key "$2" ed25519 sign never 2>/dev/null
    GNUPGHOME=$1 gpg --list-secret-keys --with-colons 2>/dev/null | awk -F: '/^fpr/ {print $10; exit}'
}

# Build a fake whoisport .deb for one architecture into $work/debs.
fake_deb() {
    local root="$work/pkg-$1"
    mkdir -p "$root/DEBIAN" "$root/usr/bin" "$work/debs"
    printf '#!/bin/sh\necho "whoisport 9.9.9"\n' >"$root/usr/bin/whoisport"
    chmod 755 "$root/usr/bin/whoisport"
    printf 'Package: whoisport\nVersion: 9.9.9-1\nArchitecture: %s\nMaintainer: test <test@example.com>\nDescription: test package\n' "$1" >"$root/DEBIAN/control"
    dpkg-deb --root-owner-group --build "$root" "$work/debs/whoisport_9.9.9-1_$1.deb" >/dev/null
}

# Run apt-get (or apt-cache) against only the test repository, as the current user.
apt_sandboxed() {
    local tool=$1 state=$2
    shift 2
    "$tool" -q -o Dir::Etc::SourceList="$state/sources.list" -o Dir::Etc::SourceParts=/nonexistent \
        -o Dir::Etc::Preferences=/dev/null -o Dir::Etc::PreferencesParts=/nonexistent \
        -o Dir::State="$state" -o Dir::State::status="$state/status" -o Dir::Cache="$state/cache" \
        -o Debug::NoLocking=1 -o APT::Sandbox::User="$(id -un)" \
        -o APT::Architecture=amd64 -o APT::Architectures::=amd64 -o APT::Architectures::=arm64 "$@"
}

# A fresh apt state directory that trusts KEYFILE for the repository at REPO.
new_state() {
    local state="$work/state-$1"
    mkdir -p "$state/lists/partial" "$state/cache/archives/partial"
    : >"$state/status"
    echo "deb [signed-by=$3] file:$2 stable main" >"$state/sources.list"
    echo "$state"
}

fake_deb amd64
fake_deb arm64
mkdir -p "$work/gpg" "$work/other-gpg"
chmod 700 "$work/gpg" "$work/other-gpg"
fingerprint=$(new_key "$work/gpg" "whoisport apt test")
GNUPGHOME="$work/gpg" "$here/apt/build.sh" "$work/debs" "$work/repo" "$fingerprint"

# --- the repository's layout ----------------------------------------------------------------
missing=""
for f in dists/stable/InRelease dists/stable/Release dists/stable/Release.gpg key.asc index.html .nojekyll \
    dists/stable/main/binary-amd64/Packages.gz dists/stable/main/binary-arm64/Packages.gz; do
    [ -e "$work/repo/$f" ] || missing="$missing $f"
done
check "builds every file apt and Pages need" "${missing:-ok}"
if grep -q 'BEGIN PGP PRIVATE' "$work/repo/key.asc"; then
    check "publishes only the public key" "key.asc holds a private key"
else
    check "publishes only the public key" ok
fi

# --- apt accepts it -------------------------------------------------------------------------
state=$(new_state good "$work/repo" "$work/repo/key.asc")
if out=$(apt_sandboxed apt-get "$state" update 2>&1) && ! grep -qE '^(W|E):' <<<"$out"; then
    check "apt update accepts the signature" ok
else
    check "apt update accepts the signature" "$out"
fi
for arch in amd64 arm64; do
    v=$(apt_sandboxed apt-cache "$state" policy "whoisport:$arch" 2>&1 | awk '/Candidate:/ {print $2}')
    check "lists whoisport for $arch" "$([ "$v" = 9.9.9-1 ] && echo ok || echo "candidate '$v'")"
done
mkdir -p "$work/dl"
if (cd "$work/dl" && apt_sandboxed apt-get "$state" download whoisport:amd64 >/dev/null 2>&1) &&
    cmp -s "$work/dl/"whoisport_9.9.9-1_amd64.deb "$work/debs/whoisport_9.9.9-1_amd64.deb"; then
    check "downloads the package intact" ok
else
    check "downloads the package intact" "download failed or differs"
fi

# --- apt refuses tampering ------------------------------------------------------------------
printf 'x' >>"$work/repo/pool/main/whoisport_9.9.9-1_amd64.deb"
rm -rf "$work/dl"/*
if (cd "$work/dl" && apt_sandboxed apt-get "$state" download whoisport:amd64 >/dev/null 2>&1); then
    check "refuses a tampered .deb" "download succeeded"
else
    check "refuses a tampered .deb" ok
fi
other=$(new_key "$work/other-gpg" "someone else")
GNUPGHOME="$work/other-gpg" gpg --batch --armor --export "$other" >"$work/other.asc"
state=$(new_state wrong-key "$work/repo" "$work/other.asc")
apt_sandboxed apt-get "$state" update >/dev/null 2>&1 || true
v=$(apt_sandboxed apt-cache "$state" policy whoisport:amd64 2>&1 | awk '/Candidate:/ {print $2}')
if [ -n "$v" ] && [ "$v" != "(none)" ]; then
    check "refuses a repository signed by another key" "apt update trusted it"
else
    check "refuses a repository signed by another key" ok
fi

if [ "$failures" -eq 0 ]; then
    echo "all apt repository tests passed"
else
    echo "$failures apt repository test(s) failed"
    exit 1
fi
