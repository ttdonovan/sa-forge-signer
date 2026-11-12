#!/usr/bin/env bash
# Install or update sa-forge-signer (verified prebuilt binary, else built from the tag) and
# sa-forge-signer-console (built from the tag), optionally connect Claude Code to it, or uninstall. --help.
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
  --uninstall            Remove the binaries from the prefix, and what --setup added (asks first).
                         Never touches the config, keys, tokens or audit log; refuses to remove
                         the signer while key files remain
  -y, --yes              With --uninstall, remove what --setup added without asking
  --dry-run              With --setup or --uninstall, print what would be done and change nothing
                         (--setup --dry-run skips the install)
  -h, --help             Show this help

Setup (after installing, connect Claude Code to the signer in one step):

  --setup                Create the agent's token file if missing (prints the [[serve.tokens]]
                         block to add to the config), add the sa-forge-signer MCP server to Claude
                         Code, and install the agent skill. Never edits the config or makes keys
  --setup-http           Add the HTTP transport with the bearer header (the token is then stored in
                         Claude Code's config) instead of the stdio relay (which stores its path)
  --scope SCOPE          claude mcp scope: user (default), local or project (local and project
                         belong to the current directory; project also puts the skill there)
  --token-file PATH      The agent's token file (default: agent.token beside the signer's config)
  --replace              Replace a sa-forge-signer MCP server that is already added
  --no-skill             Do not install the agent skill

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
setup=0
http=0
scope=user
token_file=""
replace=0
skill=1
dry=0
yes=0
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
        --setup) setup=1; shift ;;
        --setup-http) setup=1; http=1; shift ;;
        --scope) scope="${2:?--scope needs a value}"; shift 2 ;;
        --token-file) token_file="${2:?--token-file needs a value}"; shift 2 ;;
        --replace) replace=1; shift ;;
        --no-skill) skill=0; shift ;;
        --dry-run) dry=1; shift ;;
        -y | --yes) yes=1; shift ;;
        -h | --help) usage; exit 0 ;;
        *) die "unknown option: $1 (see --help)" ;;
    esac
done
bin="$prefix/bin"
case "$scope" in user | local | project) ;; *) die "--scope must be user, local or project" ;; esac
[ "$setup" = 1 ] && [ "$uninstall" = 1 ] && die "--setup and --uninstall cannot be combined"
if [ "$dry" = 1 ] && [ "$setup" = 0 ] && [ "$uninstall" = 0 ]; then
    die "--dry-run works with --setup or --uninstall"
fi
if [ "$setup" = 1 ] && [ "$dry" = 0 ] && ! have claude; then
    die "--setup needs claude (Claude Code) on PATH"
fi

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
        run rm -f "$link"
        [ "$dry" = 1 ] || say "removed $1"
    fi
}

remove() { # remove <package>: through cargo if cargo installed it, else the file
    if have cargo && cargo install --list --root "$prefix" 2>/dev/null | grep -q "^$1 "; then
        run cargo uninstall --root "$prefix" "$1"
    elif [ -e "$bin/$1" ]; then
        run rm -f "$bin/$1"
    else
        say "$1 is not installed in $bin"
        return 0
    fi
    [ "$dry" = 1 ] || say "removed $1"
}

SERVER="sa-forge-signer"
SERVE_URL="http://127.0.0.1:8790"
# Written into SKILL.md by `sa-forge-signer-mcp skill install`.
SKILL_MARK="installed by sa-forge-signer-mcp"

# show <args...>: prints a command line, quoting what the shell would split.
show() {
    local a line="+"
    for a in "$@"; do
        case "$a" in
            '' | *[!A-Za-z0-9_./:=@%+,-]*) line+=" '${a//\'/\'\\\'\'}'" ;;
            *) line+=" $a" ;;
        esac
    done
    printf '%s\n' "$line"
}

# run <args...>: runs the command, or with --dry-run only prints it.
run() {
    if [ "$dry" = 1 ]; then show "$@"; else "$@"; fi
}

# run_in <dir> <args...>: run, from dir.
run_in() {
    local dir="$1"
    shift
    if [ "$dry" = 1 ]; then
        printf '+ cd %s &&' "$dir"
        show "$@" | cut -c2-
    else
        (cd "$dir" && "$@")
    fi
}

confirm() { # confirm <question>: true only on yes from the terminal
    local answer
    { printf '%s [y/N] ' "$1" >/dev/tty; } 2>/dev/null || return 1
    { read -r answer </dev/tty; } 2>/dev/null || return 1
    case "$answer" in y | Y | yes | Yes | YES) return 0 ;; *) return 1 ;; esac
}

# What --setup added, recorded so --uninstall removes exactly that: one key=value per line.
state_file() { printf '%s/installer-setup' "$(dirname "$(config_path)")"; }
st_scope=""
st_dir=""
st_skill=""
read_state() {
    local f k v
    f="$(state_file)"
    [ -f "$f" ] || return 0
    while IFS='=' read -r k v; do
        case "$k" in
            server_scope) st_scope="$v" ;;
            server_dir) st_dir="$v" ;;
            skill_dir) st_skill="$v" ;;
        esac
    done <"$f"
}
write_state() {
    local f
    f="$(state_file)"
    if [ "$dry" = 1 ]; then
        say "would record in $f: server_scope=$st_scope server_dir=$st_dir skill_dir=$st_skill"
        return 0
    fi
    (umask 077 && mkdir -p "$(dirname "$f")")
    {
        if [ -n "$st_scope" ]; then printf 'server_scope=%s\nserver_dir=%s\n' "$st_scope" "$st_dir"; fi
        if [ -n "$st_skill" ]; then printf 'skill_dir=%s\n' "$st_skill"; fi
    } >"$f"
}

# The scope Claude Code has a sa-forge-signer server in, seen from this directory, or nothing.
registered_scope() {
    local out
    have claude || return 0
    out="$(claude mcp get "$SERVER" 2>/dev/null)" || return 0
    case "$(printf '%s\n' "$out" | sed -n 's/^ *Scope: //p' | head -1)" in
        User*) echo user ;;
        Local*) echo local ;;
        Project*) echo project ;;
        *) echo other ;;
    esac
}

# The key names in the config, space-separated.
config_keys() {
    local cfg
    cfg="$(config_path)"
    [ -r "$cfg" ] || return 0
    sed -n 's/^\[keys\.\([^]]*\)\].*/\1/p' "$cfg" | tr '\n' ' '
}

token_guidance() { # token_guidance <token-file>
    local keys
    keys="$(config_keys)"
    say "add the [[serve.tokens]] block printed above to $(config_path), and list in keys = [...]"
    say "the keys the agent may use: a session key, or a wallet key only with allow_unattended = true"
    say "(throwaway testnet keys); keys in the config now: ${keys:-none, add one with sa-forge-signer key new}"
    say "then restart sa-forge-signer-serve so it reads the token"
}

add_server() { # add_server <token-file>
    local token="$1" header
    if [ "$http" = 0 ]; then
        run claude mcp add --scope "$scope" "$SERVER" -- "$bin/sa-forge-signer-mcp" --token-file "$token"
    elif [ "$dry" = 1 ]; then
        show claude mcp add --scope "$scope" --transport http "$SERVER" "$SERVE_URL/mcp" \
            --header "Authorization: Bearer <the token in $token>"
    else
        header="Authorization: Bearer $(tr -d '[:space:]' <"$token")"
        claude mcp add --scope "$scope" --transport http "$SERVER" "$SERVE_URL/mcp" --header "$header"
    fi
}

setup_agent() {
    local token existing skills
    token="${token_file:-$(dirname "$(config_path)")/agent.token}"
    if [ "$dry" = 0 ]; then
        [ -x "$bin/sa-forge-signer" ] || die "$bin/sa-forge-signer is missing"
        if [ ! -x "$bin/sa-forge-signer-mcp" ] && { [ "$http" = 0 ] || [ "$skill" = 1 ]; }; then
            die "$bin/sa-forge-signer-mcp is missing (this release lacks it); install with --from-source"
        fi
    fi
    read_state
    # Check before changing anything, so a refusal leaves nothing half done.
    existing="$(registered_scope)"
    if [ "$existing" = other ]; then
        die "Claude Code has a $SERVER MCP server this script cannot remove; remove it with claude mcp remove"
    fi
    if [ -n "$existing" ] && [ "$replace" = 0 ] && [ "$existing" != "$scope" ]; then
        die "Claude Code already has a $SERVER MCP server in $existing scope; pass --replace to replace it"
    fi

    if [ -e "$token" ]; then
        say "using the agent token in $token"
    else
        say "creating the agent token $token"
        [ -d "$(dirname "$token")" ] || run mkdir -p -m 0700 "$(dirname "$token")"
        run "$bin/sa-forge-signer" token new agent --out "$token"
        token_guidance
    fi

    if [ "$existing" = "$scope" ] && [ "$replace" = 0 ]; then
        say "Claude Code already has a $SERVER MCP server in $scope scope; left as is (--replace to replace it)"
    else
        [ -z "$existing" ] || run claude mcp remove --scope "$existing" "$SERVER"
        add_server "$token"
        st_scope="$scope"
        st_dir="$PWD"
        write_state
    fi

    if [ "$skill" = 1 ]; then
        skills="$HOME/.claude/skills"
        local args=(skill install)
        if [ "$scope" = project ]; then
            skills="$PWD/.claude/skills"
            args+=(--project "$PWD")
        fi
        if run "$bin/sa-forge-signer-mcp" "${args[@]}"; then
            st_skill="$skills/$SERVER"
            write_state
        else
            say "the skill was not installed; the MCP server works without it"
        fi
    fi
    if [ "$http" = 0 ]; then
        say "the relay needs sa-forge-signer-serve running as the signer's user; then restart Claude Code"
    else
        say "start sa-forge-signer-serve as the signer's user, then restart Claude Code"
    fi
}

# Removes what --setup recorded, after asking unless -y.
uninstall_setup() {
    read_state
    [ -n "$st_scope$st_skill" ] || return 0
    local f
    f="$(state_file)"
    [ -z "$st_scope" ] || say "--setup added the Claude Code MCP server $SERVER ($st_scope scope, from $st_dir)"
    [ -z "$st_skill" ] || say "--setup installed the skill in $st_skill"
    if [ "$dry" = 0 ] && [ "$yes" = 0 ] && ! confirm "remove them?"; then
        say "kept them, and $f (pass -y to remove them without asking)"
        return 0
    fi
    if [ -n "$st_scope" ]; then
        if [ "$dry" = 0 ] && ! have claude; then
            say "claude is not on PATH; remove the server yourself: claude mcp remove --scope $st_scope $SERVER"
        elif [ "$dry" = 0 ] && [ ! -d "$st_dir" ]; then
            say "$st_dir is gone; remove the server yourself: claude mcp remove --scope $st_scope $SERVER"
        elif ! run_in "$st_dir" claude mcp remove --scope "$st_scope" "$SERVER"; then
            say "could not remove the MCP server $SERVER; it may have been removed already"
        fi
    fi
    if [ -n "$st_skill" ]; then
        if [ -f "$st_skill/SKILL.md" ] && grep -qF "$SKILL_MARK" "$st_skill/SKILL.md"; then
            run rm -f "$st_skill/SKILL.md" "$st_skill/references/tools.md"
            # Leaves the folders if anything else is in them.
            run rmdir "$st_skill/references" 2>/dev/null || true
            run rmdir "$st_skill" 2>/dev/null || true
            [ "$dry" = 1 ] || say "removed the skill in $st_skill"
        else
            say "left $st_skill: no SKILL.md written by sa-forge-signer-mcp there"
        fi
    fi
    run rm -f "$f"
}

if [ "$uninstall" = 1 ]; then
    uninstall_setup
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
    run rm -f "$bin/sa-forge-signer-serve" "$bin/sa-forge-signer-mcp"
    exit 0
fi

path_note() {
    case ":$PATH:" in
        *":$bin:"*) ;;
        *) say "note: $bin is not on PATH; add it to your shell profile" ;;
    esac
}

if [ "$setup" = 1 ] && [ "$dry" = 1 ]; then
    say "dry run: not installing; setup would use the binaries in $bin"
    setup_agent
    exit 0
fi

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
    [ "$setup" = 0 ] || setup_agent
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
[ "$setup" = 0 ] || setup_agent
