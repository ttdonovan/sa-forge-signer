#!/usr/bin/env bash
# One app's gate, shared by `just apps <app>-check` and the apps workflow: fmt, clippy, tests, advisories, dependency age.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
app="${1:-}"
case "$app" in
console | grant | idl-table) ;;
*)
    printf 'usage: %s <console|grant|idl-table>\n' "$0" >&2
    exit 2
    ;;
esac
cd "$ROOT/apps/$app"

cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
if [ "$app" = grant ]; then
    cargo clippy --target wasm32-unknown-unknown --locked -- -D warnings
fi
cargo test --locked
cargo audit
# The console's UI framework releases weekly and pins its own crates exactly (apps/CLAUDE.md).
young=""
if [ "$app" = console ]; then
    young="gpui-kit gpui-base gpui-component gpui-pre"
fi
ALLOW_YOUNG_PREFIXES="$young" "$ROOT/scripts/ci/crate-age.sh" "$ROOT/apps/$app/Cargo.lock"
