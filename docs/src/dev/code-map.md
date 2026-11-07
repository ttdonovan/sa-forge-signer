---
description: "Where things live: the repository layout (src, apps, docs, scripts, workflows) and each module in src/, for finding where a check, an RPC call, a route or a gate lives"
---

# Code map

## Repository

| Path | Holds |
|---|---|
| `src/` | The signer: one binary crate, mapped module by module below |
| `config.example.toml` | The annotated example config, embedded in the binary and written by `config init` |
| `apps/` | Extras a human runs beside the signer, each its own crate and workspace, kept out of the signer's build ([Building an app](apps.md)); today `apps/console/` |
| `docs/` | This book: `docs/src/` for chapters, `docs/src/dev/` for working on the signer, `docs/src/apps/` for the apps ([Writing the docs](writing-docs.md)) |
| `justfile` | Recipes: `just check` (the gate), plus the `docs` and `apps` modules in `docs/justfile` and `apps/justfile` (`just docs check`, `just apps console`) |
| `scripts/ci/` | `check.sh` (the gate run by `just check` and CI), `crate-age.sh` (dependency age), `docs-check.sh` (`just docs check`), `docker-toolchain.sh` (the Dockerfile's Rust matches `rust-toolchain.toml`) |
| `scripts/installer/` | `install.sh`: installs, updates or uninstalls the signer and the console on a host ([Install and update](../install.md)) |
| `scripts/skills/` | `gpui-kit.sh`: installs GPUI Kit's Claude skills at the console's `gpui-kit` version ([GPUI Kit skills](../appendix/gpui-kit-skills.md)) |
| `scripts/git-review/` | The local review workflow, `just mr-open` to `just mr-approve` ([Local review with git-review](../appendix/git-review.md)) |
| `.github/workflows/` | `ci.yml` (the gate and a Docker build), `pages.yml` (publishes this book), `release-plz.yml` (release PRs and tags), `release-binaries.yml` (attested binaries for each tag) |
| `Dockerfile` | The container image ([Running in Docker](../docker.md)) |
| `PLAN.md`, `CHANGELOG.md`, `CONTRIBUTING.md`, `SECURITY.md` | Included in this book as [Plan](plan.md), [Changelog](changelog.md), [Contributing](contributing.md) and [Security](../security.md) |

## Modules

One binary crate; each module's `//!` comment is the authority if this page drifts.

| Module | Owns |
|---|---|
| `main.rs` | The CLI (`check`, `sign`, `key`, `token`, `audit`, `serve`, `config`) and exit codes |
| `config.rs` | Loading and validating `config.toml`: keys, classes, approval, limits, serve tokens; `config init` and `config show` |
| `cluster.rs` | Built-in cluster profiles: genesis hash, default RPC, explorer, and the programs any key may call |
| `payload.rs` | The forge-mcp `build_*` payload (version 1): an instruction list, not a serialized transaction |
| `transfer.rs` | Funding payloads: a plain System Program transfer in the same shape forge-mcp returns |
| `checks.rs` | Structural checks on the compiled message; none decode game instructions |
| `engine.rs` | The check and sign pipeline: simulate, limits, sign, send, confirm |
| `rpc.rs` | The minimal Solana JSON-RPC client: only the calls the signer needs |
| `keystore.rs` | The file key store: `<key_dir>/<name>.json`, 0700 directory, 0600 files; key create, import and destroy |
| `audit.rs` | The append-only, hash-chained JSONL audit log, and the per-key signing lock |
| `serve.rs` | `serve`: JSON over HTTP at `/v1/check` and `/v1/sign`, MCP at `/mcp`, bearer-token auth |

Checks and gates: `just check` runs `scripts/ci/check.sh` (fmt, clippy, tests, shellcheck, advisories, dependency age), the same gate as CI.
