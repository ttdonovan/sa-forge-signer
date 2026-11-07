---
description: "Each module in src/ and what it owns, for finding where a check, an RPC call or a route lives"
---

# Code map

One binary crate; each module's `//!` comment is the authority if this page drifts.

| Module | Owns |
|---|---|
| `main.rs` | The CLI (`check`, `sign`, `key`, `token`, `audit`, `serve`) and exit codes |
| `config.rs` | Loading and validating `config.toml`: keys, classes, approval, limits, serve tokens |
| `cluster.rs` | Built-in cluster profiles: genesis hash, default RPC, explorer, and the programs any key may call |
| `payload.rs` | The forge-mcp `build_*` payload (version 1): an instruction list, not a serialized transaction |
| `transfer.rs` | Funding payloads: a plain System Program transfer in the same shape forge-mcp returns |
| `checks.rs` | Structural checks on the compiled message; none decode game instructions |
| `engine.rs` | The check and sign pipeline: simulate, limits, sign, send, confirm |
| `rpc.rs` | The minimal Solana JSON-RPC client: only the calls the signer needs |
| `keystore.rs` | The file key store: `<key_dir>/<name>.json`, 0700 directory, 0600 files |
| `audit.rs` | The append-only, hash-chained JSONL audit log, and the per-key signing lock |
| `serve.rs` | `serve`: JSON over HTTP at `/v1/check` and `/v1/sign`, MCP at `/mcp`, bearer-token auth |

Checks and gates: `just check` runs `scripts/ci/check.sh` (fmt, clippy, tests, shellcheck, advisories, dependency age), the same gate as CI.
