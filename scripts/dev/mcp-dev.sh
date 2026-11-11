#!/usr/bin/env bash
# Run the debug sa-forge-signer-serve that this repository's .mcp.json talks to, on a throwaway
# home in target/dev (its own config, key, token and audit log) and its own port. Never a real key:
# a Claude session here runs as the user who owns that home, so nothing isolates it.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
home="$ROOT/target/dev"
config="$home/config.toml"
token="$home/agent.token"
listen="127.0.0.1:18790"
bin="$ROOT/target/debug"

for b in sa-forge-signer sa-forge-signer-serve sa-forge-signer-mcp; do
    [ -x "$bin/$b" ] || {
        printf 'error: %s is not built; run "just mcp-build" first\n' "$bin/$b" >&2
        exit 1
    }
done

if [ ! -f "$config" ]; then
    mkdir -p "$home"
    printf '# Throwaway config for debugging the MCP route; written by scripts/dev/mcp-dev.sh.\n' >"$config"
    pubkey="$("$bin/sa-forge-signer" --config "$config" key new dev --class session 2>/dev/null)"
    rm -f "$token"
    hash="$("$bin/sa-forge-signer" --config "$config" token new debug --out "$token" 2>/dev/null)"
    cat >>"$config" <<CONFIG

[keys.dev]
class = "session"
pubkey = "$pubkey"

[serve]
listen = "$listen"

[[serve.tokens]]
name = "debug"
sha256 = "$hash"
keys = ["dev"]
CONFIG
    printf '==> wrote %s with a new unfunded session key "dev" (%s)\n' "$config" "$pubkey" >&2
fi

printf '==> serving on %s; .mcp.json reads the token from %s\n' "$listen" "$token" >&2
exec "$bin/sa-forge-signer-serve" --config "$config"
