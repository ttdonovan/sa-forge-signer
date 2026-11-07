#!/usr/bin/env bash
# Install or update sa-forge-signer (verified prebuilt binary, else built from the tag) and
# sa-forge-signer-console (built from the tag) on this host. Run with --help for options.
set -euo pipefail

REPO="ttdonovan/sa-forge-signer"
URL="https://github.com/$REPO"

usage() {
    cat <<'EOF'
Usage: install.sh [options]

Installs or updates sa-forge-signer and sa-forge-signer-console from a release tag.

  --tag vX.Y.Z           Release to install (default: the latest tag)
  --prefix DIR           Install into DIR/bin (default: ~/.local)
  --from-source          Build the signer from the tag instead of downloading the prebuilt binary
  --no-console           Install only the signer
  --console-ref BRANCH   Build the console from a branch instead of the tag (for an unreleased console)
  --skip-attestation     Check only SHA256SUMS, not the build provenance (no gh needed)
  --force                Reinstall even if the installed version matches
  -h, --help             Show this help

The prebuilt signer is verified against SHA256SUMS and its build provenance attestation (needs gh).
The console has no prebuilt binary; it is built with cargo (Rust 1.95 or later).
EOF
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}
say() { printf '==> %s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }

tag=""
prefix="$HOME/.local"
from_source=0
console=1
console_ref=""
attest=1
force=0
while [ $# -gt 0 ]; do
    case "$1" in
        --tag) tag="${2:?--tag needs a value}"; shift 2 ;;
        --prefix) prefix="${2:?--prefix needs a value}"; shift 2 ;;
        --from-source) from_source=1; shift ;;
        --no-console) console=0; shift ;;
        --console-ref) console_ref="${2:?--console-ref needs a value}"; shift 2 ;;
        --skip-attestation) attest=0; shift ;;
        --force) force=1; shift ;;
        -h | --help) usage; exit 0 ;;
        *) die "unknown option: $1 (see --help)" ;;
    esac
done
bin="$prefix/bin"

# The release target for this host, or empty if there is no prebuilt binary for it.
host_target() {
    case "$(uname -s)-$(uname -m)" in
        Darwin-arm64) echo aarch64-apple-darwin ;;
        Darwin-x86_64) echo x86_64-apple-darwin ;;
        Linux-x86_64) echo x86_64-unknown-linux-musl ;;
        Linux-aarch64 | Linux-arm64) echo aarch64-unknown-linux-musl ;;
        *) echo "" ;;
    esac
}

latest_tag() {
    git ls-remote --tags --refs --sort=-version:refname "$URL" 'v*' | head -1 | sed 's#.*refs/tags/##'
}

# Prints the installed signer's version as vX.Y.Z, or nothing.
installed_signer() {
    [ -x "$bin/sa-forge-signer" ] || return 0
    "$bin/sa-forge-signer" --version 2>/dev/null | awk '{print "v" $2}'
}

download() { # download <asset> <dir>
    if have gh; then
        gh release download "$tag" --repo "$REPO" --pattern "$1" --dir "$2" --clobber
    else
        curl -fsSL --retry 3 -o "$2/$1" "$URL/releases/download/$tag/$1"
    fi
}

install_prebuilt() { # install_prebuilt <target>
    local target="$1" archive="sa-forge-signer-$tag-$1.tar.gz" tmp
    tmp="$(mktemp -d)"
    # shellcheck disable=SC2064 # expand tmp now
    trap "rm -rf '$tmp'" RETURN
    say "downloading $archive"
    download "$archive" "$tmp"
    download SHA256SUMS "$tmp"
    local sum=(shasum -a 256)
    have sha256sum && sum=(sha256sum)
    (cd "$tmp" && grep " $archive\$" SHA256SUMS | "${sum[@]}" -c -) >/dev/null ||
        die "checksum mismatch for $archive"
    say "checksum ok"
    if [ "$attest" = 1 ]; then
        have gh || die "gh is needed to verify the build provenance; install it, or pass --skip-attestation"
        gh attestation verify "$tmp/$archive" --repo "$REPO" \
            --signer-workflow "$REPO/.github/workflows/release-binaries.yml" >/dev/null ||
            die "attestation verification failed for $archive"
        say "build provenance ok"
    fi
    tar -xzf "$tmp/$archive" -C "$tmp"
    local exe="$tmp/${archive%.tar.gz}/sa-forge-signer"
    [ -f "$exe" ] || die "$archive has no sa-forge-signer binary"
    install -m 0755 "$exe" "$bin/sa-forge-signer"
}

cargo_install() { # cargo_install <package> <ref-args...>
    have cargo || die "cargo is needed to build $1 (https://rustup.rs), or pass --no-console"
    local pkg="$1"
    shift
    say "building $pkg with cargo"
    local args=(install --locked --git "$URL" "$@" --root "$prefix" "$pkg")
    [ "$force" = 1 ] && args+=(--force)
    cargo "${args[@]}"
}

[ -n "$tag" ] || tag="$(latest_tag)"
[ -n "$tag" ] || die "could not find a release tag at $URL"
mkdir -p "$bin"
say "release $tag, installing into $bin"

current="$(installed_signer)"
if [ "$current" = "$tag" ] && [ "$force" = 0 ]; then
    say "sa-forge-signer $tag is already installed"
else
    target="$(host_target)"
    if [ "$from_source" = 1 ] || [ -z "$target" ]; then
        [ -n "$target" ] || say "no prebuilt binary for $(uname -s) $(uname -m)"
        cargo_install sa-forge-signer --tag "$tag"
    else
        install_prebuilt "$target"
    fi
    say "installed $("$bin/sa-forge-signer" --version)${current:+ (was $current)}"
fi

if [ "$console" = 1 ]; then
    if [ -n "$console_ref" ]; then
        cargo_install sa-forge-signer-console --branch "$console_ref"
    elif curl -fsI "https://raw.githubusercontent.com/$REPO/$tag/apps/console/Cargo.toml" >/dev/null 2>&1; then
        # cargo skips the build when this tag is already installed
        cargo_install sa-forge-signer-console --tag "$tag"
    else
        say "skipping the console: $tag does not include it (pass --console-ref <branch> to build an unreleased one)"
    fi
fi

case ":$PATH:" in
    *":$bin:"*) ;;
    *) say "note: $bin is not on PATH; add it to your shell profile" ;;
esac
say "config, keys and audit log: ~/.config/sa-forge-signer/ (see the book's Configuration chapter)"
