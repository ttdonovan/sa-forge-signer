# Plan

Work tracked in this repository. Update it in the same change as the work it tracks.

## Next

1. **Cargo's native minimum publish age (not before 2026-11-19).** Rust 1.100 (due 2026-11-12) stabilises `registry.global-min-publish-age`; on 1.98 the key only warns. Wait until the release is itself at least 7 days old (later if a point release follows), then:
   - bump `rust-toolchain.toml` to 1.100.x, and the `Dockerfile`'s `RUST_IMAGE` tag and digest to match (`scripts/ci/docker-toolchain.sh` fails until they agree);
   - add `.cargo/config.toml` with `[registry]` `global-min-publish-age = "7 days"`;
   - remove `scripts/ci/crate-age.sh` and its line in `scripts/ci/check.sh`, and drop "dependency age" from the `check` recipe comment in the justfile (Cargo enforces it at resolve time instead);
   - confirm `cargo update` then respects the age and `just check` and CI still pass.

## Backlog

- **`check` previews the sign-only checks:** today `check` skips the rate limit, the daily cap and approval, so a clean `check` can still be refused by `sign`. Evaluate the limits (they only read the audit log) and refuse a `deny` key in `check` too, and note in the report when a `confirm` key would need a human to approve.
- **Approval outside the terminal:** `confirm` reads `/dev/tty`, which an agent running as the terminal's user can type into through a multiplexer or a terminal app's scripting API, and which `serve` in a container usually lacks. Move approval to a channel that checks who answers, such as a Unix socket that accepts only a configured approver uid (peer credentials), so `confirm` keys also work under `serve`.
- **`serve` thread cap:** one thread per request is unbounded today; cap concurrent requests and answer the rest with 503.
- **SPL Token authority check:** refuse top-level SPL Token `Transfer` / `Approve` (and their `Checked` forms) whose authority is the signing key, using the public SPL Token instruction layout. Today such instructions pass on program id alone, so a hand-written payload could move tokens from a token account the key owns.
- **Key stores:** an encrypted-file backend (age, passphrase at `serve` start) and macOS Keychain / Secret Service backends, for running outside a container. Check first whether a Keychain item can be isolated from other processes of the same user.
- **Secrets in memory:** `Keypair` already zeroes itself on drop (`solana-ed25519`'s default `zeroize` feature), and key files, payload text and partial signers are read into `Zeroizing` buffers. What is left: buffers that grow while being filled (`serde_json` parsing a key file into a `Vec<u8>`, `read_to_string` from stdin or an HTTP body) free copies that are never wiped, so reserve the full size up front; `serve`'s MCP route parses the request into a `serde_json::Value` that holds `partial_signers` and is dropped without wiping; and moves of a `Keypair` leave copies on the stack. Ahead of those, guard against copies outside the process: turn off core dumps (`RLIMIT_CORE` 0, and `PR_SET_DUMPABLE` 0 on Linux) and consider `mlock` for key pages so they are not swapped out. Low priority: all of this assumes a memory disclosure in a process the agent's OS user cannot read.
- **Local-validator integration tests:** prove the outcomes that only fake-chain unit tests cover today: `failed` (exit 12, landed with a program error), an expired blockhash with no status found (`unknown`, exit 14; exit 13 is retired), and the one-time re-sign on `BlockhashNotFound`.
- **Releases:** reproducible builds (verify two builds match byte for byte), an SBOM attested next to the provenance, and an external review before any mainnet use. Checksums and build provenance attestations ship with each release since v0.4.0.
