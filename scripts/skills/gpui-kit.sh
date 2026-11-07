#!/usr/bin/env bash
# Install (or update, or remove) GPUI Kit's Claude skills, gpui-kit and gpui-kit-design-guides, at
# the gpui-kit version the console builds against. Optional: for people and agents working on the
# console's UI. Third-party content from github.com/longbridge/gpui-kit; read it before use.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
URL="https://github.com/longbridge/gpui-kit"
SKILLS=(gpui-kit gpui-kit-design-guides)
MARKER=".installed-by-sa-forge-signer"

usage() {
    cat <<'EOF'
Usage: gpui-kit.sh [options]

Installs GPUI Kit's skills (gpui-kit, gpui-kit-design-guides) from the release tag matching the
gpui-kit version in apps/console/Cargo.lock. Claude Code loads them in its next session.

  --user          Install into ~/.claude/skills (default)
  --project       Install into this repository's .claude/skills (ignored by git)
  --dest DIR      Install into DIR
  --tag vX.Y.Z    Use this tag instead of the console's version
  --uninstall     Remove the skills this script installed in the destination
  -h, --help      Show this help

It never replaces a skill folder it did not install: those carry no marker file.
EOF
}

die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}
say() { printf '==> %s\n' "$*"; }

dest="$HOME/.claude/skills"
tag=""
uninstall=0
while [ $# -gt 0 ]; do
    case "$1" in
        --user) dest="$HOME/.claude/skills"; shift ;;
        --project) dest="$ROOT/.claude/skills"; shift ;;
        --dest) dest="${2:?--dest needs a value}"; shift 2 ;;
        --tag) tag="${2:?--tag needs a value}"; shift 2 ;;
        --uninstall) uninstall=1; shift ;;
        -h | --help) usage; exit 0 ;;
        *) die "unknown option: $1 (see --help)" ;;
    esac
done

# ours <dir>: whether dir is a skill folder this script installed.
ours() { [ -f "$1/$MARKER" ]; }

if [ "$uninstall" = 1 ]; then
    for s in "${SKILLS[@]}"; do
        if ours "$dest/$s"; then
            rm -rf "${dest:?}/$s"
            say "removed $dest/$s"
        elif [ -e "$dest/$s" ]; then
            say "kept $dest/$s: not installed by this script"
        fi
    done
    exit 0
fi

if [ -z "$tag" ]; then
    lock="$ROOT/apps/console/Cargo.lock"
    [ -f "$lock" ] || die "$lock not found; pass --tag"
    version="$(awk '/^name = "gpui-kit"$/ { getline; gsub(/"/, "", $3); print $3; exit }' "$lock")"
    [ -n "$version" ] || die "no gpui-kit in $lock; pass --tag"
    tag="v$version"
fi

for s in "${SKILLS[@]}"; do
    if [ -e "$dest/$s" ] && ! ours "$dest/$s"; then
        die "$dest/$s exists and was not installed by this script; remove it or pass --dest"
    fi
done

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
say "fetching skills/ from $URL at $tag"
# git's messages are shown only on failure (an annotated tag also warns "is not a commit").
if ! git -c advice.detachedHead=false clone --quiet --depth 1 --filter=blob:none --sparse \
    --branch "$tag" "$URL" "$tmp/repo" 2>"$tmp/git.log" ||
    ! git -C "$tmp/repo" sparse-checkout set skills 2>>"$tmp/git.log"; then
    cat "$tmp/git.log" >&2
    die "could not fetch skills/ at $tag from $URL"
fi
commit="$(git -C "$tmp/repo" rev-parse HEAD)"

mkdir -p "$dest"
for s in "${SKILLS[@]}"; do
    src="$tmp/repo/skills/$s"
    [ -f "$src/SKILL.md" ] || die "$tag has no skills/$s/SKILL.md"
    rm -rf "${dest:?}/$s"
    cp -R "$src" "$dest/$s"
    printf 'source: %s/tree/%s/skills/%s\ncommit: %s\n' "$URL" "$tag" "$s" "$commit" >"$dest/$s/$MARKER"
    say "installed $dest/$s ($tag, $commit)"
done
say "Claude Code loads them in its next session; they are third-party instructions, so read SKILL.md first"
