#!/usr/bin/env bash
# Install or update sa-forge-signer (verified prebuilt binary, else built from the tag) and
# sa-forge-signer-console (built from the tag) on this host, or uninstall them. Run with --help.
set -euo pipefail

REPO="ttdonovan/sa-forge-signer"
URL="https://github.com/$REPO"

usage() {
    cat <<'EOF'
Usage: install.sh [options]

Installs or updates sa-forge-signer (with sa-forge-signer-serve and sa-forge-signer-mcp) and
sa-forge-signer-console from a release tag.

  --tag vX.Y.Z           Release to install (default: the latest tag)
  --prefix DIR           Install into DIR/bin (default: ~/.local)
  --from-source          Build the signer from the tag instead of downloading the prebuilt binary
  --from-checkout        Build both from the repository this script is in (its working tree,
                         uncommitted changes included) instead of a release; ignores --tag
  --no-console           Install only the signer
  --console-ref BRANCH   Build the console from a branch instead of the tag (for an unreleased console)
  --skip-attestation     Check only SHA256SUMS, not the build provenance (no gh needed)
  --force                Reinstall even if the installed version matches; with --uninstall,
                         remove the signer even though key files remain
  --no-alias             Do not link sfs, sfs-serve, sfs-mcp and sfs-console to the binaries in the
                         same directory
  --uninstall            Remove the binaries from the prefix. Never touches the config, keys or
                         audit log; refuses to remove the signer while key files remain
  -h, --help             Show this help

The prebuilt signer is verified against SHA256SUMS and its build provenance attestation (needs gh).
The console has no prebuilt binary; it is built with cargo (Rust 1.95 or later; on macOS, macOS 15+ and the Xcode Command Line Tools).
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
from_checkout=0
console=1
console_ref=""
attest=1
force=0
uninstall=0
alias=1
while [ $# -gt 0 ]; do
    case "$1" in
        --tag) tag="${2:?--tag needs a value}"; shift 2 ;;
        --prefix) prefix="${2:?--prefix needs a value}"; shift 2 ;;
        --from-source) from_source=1; shift ;;
        --from-checkout) from_checkout=1; shift ;;
        --no-console) console=0; shift ;;
        --console-ref) console_ref="${2:?--console-ref needs a value}"; shift 2 ;;
        --skip-attestation) attest=0; shift ;;
        --force) force=1; shift ;;
        --uninstall) uninstall=1; shift ;;
        --no-alias) alias=0; shift ;;
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
    # Older releases lack serve or mcp; do not leave a newer one beside an older signer.
    local extra
    for extra in serve mcp; do
        if [ -f "$exe-$extra" ]; then
            install -m 0755 "$exe-$extra" "$bin/sa-forge-signer-$extra"
        else
            rm -f "$bin/sa-forge-signer-$extra"
        fi
    done
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

# The signer's config path, as the signer resolves it.
config_path() { printf '%s' "${SA_FORGE_SIGNER_CONFIG:-$HOME/.config/sa-forge-signer/config.toml}"; }

# The config's key_dir (top-level, before any table), resolved against the config's directory.
key_dir() {
    local cfg dir kd=""
    cfg="$(config_path)"
    dir="$(dirname "$cfg")"
    [ -f "$cfg" ] && kd="$(awk '/^\[/ { exit } /^key_dir *=/ { sub(/^[^"]*"/, ""); sub(/".*/, ""); print }' "$cfg")"
    kd="${kd:-keys}"
    case "$kd" in /*) printf '%s' "$kd" ;; *) printf '%s/%s' "$dir" "$kd" ;; esac
}

# link_alias <link> <target>: links bin/link -> target; never replaces a link or file it did not make.
link_alias() {
    [ "$alias" = 1 ] && [ -e "$bin/$2" ] || return 0
    local link="$bin/$1" found
    if [ -L "$link" ] && [ "$(readlink "$link")" = "$2" ]; then
        :
    elif [ -e "$link" ] || [ -L "$link" ]; then
        say "not linking $1: $link already exists"
        return 0
    else
        ln -s "$2" "$link"
        say "linked $1 -> $2"
    fi
    found="$(command -v "$1" || true)"
    if [ -n "$found" ] && [ "$found" != "$link" ]; then
        say "note: $found comes first on PATH, so $1 runs that, not $2"
    fi
}

# unlink_alias <link> <target>: removes bin/link only if it is the link link_alias made.
unlink_alias() {
    local link="$bin/$1"
    if [ -L "$link" ] && [ "$(readlink "$link")" = "$2" ]; then
        rm -f "$link"
        say "removed $1"
    fi
}

remove() { # remove <package>: through cargo if cargo installed it, else the file
    if have cargo && cargo install --list --root "$prefix" 2>/dev/null | grep -q "^$1 "; then
        cargo uninstall --root "$prefix" "$1"
    elif [ -e "$bin/$1" ]; then
        rm -f "$bin/$1"
    else
        say "$1 is not installed in $bin"
        return 0
    fi
    say "removed $1"
}

if [ "$uninstall" = 1 ]; then
    unlink_alias sfs-console sa-forge-signer-console
    remove sa-forge-signer-console
    kd="$(key_dir)"
    keys=()
    for f in "$kd"/*.json; do
        [ -e "$f" ] && keys+=("$(basename "$f" .json)")
    done
    if [ "${#keys[@]}" -gt 0 ]; then
        say "key files remain in $kd: ${keys[*]}"
        say "the config, keys and audit log are never removed by this script; to remove them yourself:"
        say "  1. revoke each key's grant on-chain (forge-mcp build_remove_profile_key, signed by the profile authority)"
        for k in "${keys[@]}"; do
            say "  2. sa-forge-signer key destroy $k   (refuses while the key holds ZINK)"
        done
        say "  3. rm -r $(dirname "$(config_path)")"
        if [ "$force" = 0 ]; then
            die "kept sa-forge-signer: key destroy needs it; run the steps above, then --uninstall again (or add --force)"
        fi
    fi
    unlink_alias sfs-mcp sa-forge-signer-mcp
    unlink_alias sfs-serve sa-forge-signer-serve
    unlink_alias sfs sa-forge-signer
    # cargo removes all three with the package; a prebuilt install leaves serve and mcp to remove here
    remove sa-forge-signer
    rm -f "$bin/sa-forge-signer-serve" "$bin/sa-forge-signer-mcp"
    exit 0
fi

path_note() {
    case ":$PATH:" in
        *":$bin:"*) ;;
        *) say "note: $bin is not on PATH; add it to your shell profile" ;;
    esac
}

if [ "$from_checkout" = 1 ]; then
    root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
    if [ ! -f "$root/Cargo.toml" ] || [ ! -d "$root/src" ]; then
        die "$root is not a sa-forge-signer checkout"
    fi
    have cargo || die "cargo is needed to build from the checkout (https://rustup.rs)"
    mkdir -p "$bin"
    say "building from $root ($(git -C "$root" describe --tags --always --dirty 2>/dev/null || echo "not a git checkout")), installing into $bin"
    cargo install --locked --force --path "$root" --root "$prefix"
    say "installed $("$bin/sa-forge-signer" --version)"
    link_alias sfs sa-forge-signer
    link_alias sfs-serve sa-forge-signer-serve
    link_alias sfs-mcp sa-forge-signer-mcp
    if [ "$console" = 1 ]; then
        if [ -f "$root/apps/console/Cargo.toml" ]; then
            cargo install --locked --force --path "$root/apps/console" --root "$prefix"
            say "installed sa-forge-signer-console"
            link_alias sfs-console sa-forge-signer-console
        else
            say "skipping the console: this checkout does not include it"
        fi
    fi
    path_note
    say "config, keys and audit log: ~/.config/sa-forge-signer/ (see the book's Configuration chapter)"
    exit 0
fi

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
link_alias sfs sa-forge-signer
link_alias sfs-serve sa-forge-signer-serve
link_alias sfs-mcp sa-forge-signer-mcp

if [ "$console" = 1 ]; then
    if [ -n "$console_ref" ]; then
        cargo_install sa-forge-signer-console --branch "$console_ref"
    elif curl -fsI "https://raw.githubusercontent.com/$REPO/$tag/apps/console/Cargo.toml" >/dev/null 2>&1; then
        # cargo skips the build when this tag is already installed
        cargo_install sa-forge-signer-console --tag "$tag"
    else
        say "skipping the console: $tag does not include it (pass --console-ref <branch> to build an unreleased one)"
    fi
    link_alias sfs-console sa-forge-signer-console
fi

path_note
say "config, keys and audit log: ~/.config/sa-forge-signer/ (see the book's Configuration chapter)"
