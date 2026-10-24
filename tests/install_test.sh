#!/bin/sh
# Tests for install.sh. Run with:  sh tests/install_test.sh
#
# No network: a fake release (a tiny shell script posing as the whoisport binary, packed
# the way real releases are) is served from a temporary directory through file:// URLs.
# Covers: a normal install, a tampered download (must install nothing), a missing checksum
# file (must refuse), version-tag handling, target detection, and the PATH hint.

set -eu

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

# --- a fake release for this machine's own target ------------------------------------
WHOISPORT_INSTALL_SOURCED=1 . "$here/install.sh"
target=$(detect_target)
tag="v9.9.9"
rel="$work/releases/download/$tag"
mkdir -p "$rel" "$work/pack"
printf '#!/bin/sh\necho "whoisport 9.9.9"\n' >"$work/pack/whoisport"
chmod 755 "$work/pack/whoisport"
archive="whoisport-$tag-$target.tar.gz"
tar -czf "$rel/$archive" -C "$work/pack" whoisport
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$rel" && sha256sum "$archive" >SHA256SUMS)
else
    (cd "$rel" && shasum -a 256 "$archive" >SHA256SUMS)
fi
export WHOISPORT_BASE_URL="file://$work/releases"

# --- 1. a normal install -------------------------------------------------------------
dest="$work/bin1"
out=$(sh "$here/install.sh" --version "$tag" --dir "$dest" 2>&1) || out="exit $?: $out"
if [ -x "$dest/whoisport" ] && [ "$("$dest/whoisport" --version)" = "whoisport 9.9.9" ]; then
    check "installs and the binary runs" ok
else
    check "installs and the binary runs" "$out"
fi
case "$out" in *"checksum OK"*) check "reports the checksum check" ok ;; *) check "reports the checksum check" "$out" ;; esac

# --- 2. version without the leading v is accepted ------------------------------------
dest="$work/bin2"
if sh "$here/install.sh" --version 9.9.9 --dir "$dest" >/dev/null 2>&1 && [ -x "$dest/whoisport" ]; then
    check "accepts a version without the v" ok
else
    check "accepts a version without the v" "install failed"
fi

# --- 3. environment variables work like the options ----------------------------------
dest="$work/bin3"
if WHOISPORT_VERSION="$tag" WHOISPORT_INSTALL_DIR="$dest" sh "$here/install.sh" >/dev/null 2>&1 && [ -x "$dest/whoisport" ]; then
    check "environment variables work" ok
else
    check "environment variables work" "install failed"
fi

# --- 4. a tampered download installs nothing -----------------------------------------
cp -r "$rel" "$work/good"
printf 'tampered' >>"$rel/$archive"
dest="$work/bin4"
if out=$(sh "$here/install.sh" --version "$tag" --dir "$dest" 2>&1); then
    check "rejects a tampered download" "installer succeeded: $out"
elif [ -e "$dest/whoisport" ]; then
    check "rejects a tampered download" "a binary was installed anyway"
else
    case "$out" in *"checksum mismatch"*) check "rejects a tampered download" ok ;; *) check "rejects a tampered download" "wrong message: $out" ;; esac
fi
cp "$work/good/$archive" "$rel/$archive"

# --- 5. a missing SHA256SUMS refuses to install --------------------------------------
mv "$rel/SHA256SUMS" "$work/SHA256SUMS.bak"
dest="$work/bin5"
if out=$(sh "$here/install.sh" --version "$tag" --dir "$dest" 2>&1); then
    check "refuses without SHA256SUMS" "installer succeeded: $out"
elif [ -e "$dest/whoisport" ]; then
    check "refuses without SHA256SUMS" "a binary was installed anyway"
else
    case "$out" in *"unverified"*) check "refuses without SHA256SUMS" ok ;; *) check "refuses without SHA256SUMS" "wrong message: $out" ;; esac
fi
mv "$work/SHA256SUMS.bak" "$rel/SHA256SUMS"

# --- 6. a release that does not exist ------------------------------------------------
if out=$(sh "$here/install.sh" --version v0.0.1 --dir "$work/bin6" 2>&1); then
    check "fails clearly for an unknown release" "installer succeeded"
else
    case "$out" in *"could not download"*) check "fails clearly for an unknown release" ok ;; *) check "fails clearly for an unknown release" "$out" ;; esac
fi

# --- 7. bad options ------------------------------------------------------------------
if out=$(sh "$here/install.sh" --bogus 2>&1); then
    check "rejects an unknown option" "installer succeeded"
else
    case "$out" in *"unknown option"*) check "rejects an unknown option" ok ;; *) check "rejects an unknown option" "$out" ;; esac
fi
out=$(sh "$here/install.sh" --help 2>&1) || true
case "$out" in *"--version"*"--dir"*) check "--help lists the options" ok ;; *) check "--help lists the options" "$out" ;; esac

# --- 8. the PATH hint ----------------------------------------------------------------
dest="$work/bin8"
out=$(PATH="/usr/bin:/bin" sh "$here/install.sh" --version "$tag" --dir "$dest" 2>&1) || true
case "$out" in *"not on your PATH"*) check "warns when the directory is not on PATH" ok ;; *) check "warns when the directory is not on PATH" "$out" ;; esac
out=$(PATH="$dest:/usr/bin:/bin" sh "$here/install.sh" --version "$tag" --dir "$dest" 2>&1) || true
case "$out" in *"not on your PATH"*) check "stays quiet when it is on PATH" "$out" ;; *) check "stays quiet when it is on PATH" ok ;; esac

# --- 9. target detection (uname is replaced by a function for each case) -------------
while read -r fake_os fake_arch want; do
    got=$(
        uname() { if [ "$1" = "-s" ]; then echo "$fake_os"; else echo "$fake_arch"; fi; }
        sysctl() { echo 0; }
        detect_target
    )
    if [ "$got" = "$want" ]; then check "detects $fake_os $fake_arch" ok; else check "detects $fake_os $fake_arch" "got $got, want $want"; fi
done <<'CASES'
Linux x86_64 x86_64-unknown-linux-musl
Linux aarch64 aarch64-unknown-linux-musl
Darwin arm64 aarch64-apple-darwin
Darwin x86_64 x86_64-apple-darwin
CASES
if out=$(uname() { if [ "$1" = "-s" ]; then echo FreeBSD; else echo x86_64; fi; }; detect_target 2>&1); then
    check "rejects an unsupported OS" "accepted"
else
    check "rejects an unsupported OS" ok
fi

if [ "$failures" -eq 0 ]; then
    echo "all installer tests passed"
else
    echo "$failures installer test(s) failed"
    exit 1
fi
