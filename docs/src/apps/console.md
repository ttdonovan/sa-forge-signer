---
description: "sa-forge-signer-console: a read-only desktop view of serve health, the audit log and per-key limits, run locally or against a signer in Docker. Early spike."
---

# Console

`sa-forge-signer-console` is a desktop window (egui) for watching a signer. It reads the signer's audit log and config and polls `serve`'s `/health`; it never writes, locks, signs or approves anything. It is an early spike in `apps/console/`.

```sh
just apps console                                   # the signer's default config
just apps console --config ./config.toml --state-dir ./state --url http://127.0.0.1:8790
```

| Flag | Default |
|---|---|
| `--config` | `$SA_FORGE_SIGNER_CONFIG`, else `~/.config/sa-forge-signer/config.toml`, as the signer |
| `--state-dir` | the config's `state_dir` (holds `audit.jsonl`) |
| `--url` | `http://` plus the config's `[serve] listen` (`0.0.0.0` becomes `127.0.0.1`); none without a `[serve]` table |

## What it shows

- **Status bar:** `serve` up or down, the audit log's hash chain (intact, or the first line that breaks it), and how many sends are unresolved.
- **Keys:** each key's class and approval, signs in the last minute against its rate limit, and ZINK spent or reserved in 24 h against its daily cap, as the signer computes them (see [How it works](../how-it-works.md)). Click a key to filter the log.
- **Audit log:** newest first. Unresolved sends (an orphan intent, or an `unknown` outcome) are highlighted; reconcile them before retrying (see [Retrying safely](../check-and-sign.md#retrying-safely)). Click a row for every field of the entry.

It re-reads `audit.jsonl` about once a second when its size or modification time changes, and skips a last line still being written. The config supplies key classes, approval and limits; a key that appears only in the log is shown with the default limits. The config holds no secrets, only token hashes.

## Access

The console runs as the human, who needs **read** access to the signer's `state_dir` and config. Give read access only (group read or a read-only mount): write access to `state_dir` lets its holder rewrite the audit chain (see [Security](../security.md)). The agent's OS user must have neither.

## Watching a signer in Docker

Run the console on the host against the container from [Running in Docker](../docker.md): health comes over the published port, and the audit log from the state mount. The config's `state_dir` is a path inside the container, so pass the host side of the mount:

```sh
just apps console --config "$PWD/config.toml" --state-dir "$PWD/state"
```

`listen = "0.0.0.0:8790"` becomes `http://127.0.0.1:8790`, which matches `-p 127.0.0.1:8790:8790`; pass `--url` for any other port. `/health` needs no token. The state must be a bind mount: a named volume has no host path to read.

On macOS with Docker Desktop, files the container writes to a bind mount belong to your macOS user on the host (uid 10001 only inside the container), so the console can read them without extra permissions. The same holds for the keys mount; see [Security](../security.md).

## Working on it

The crate has its own workspace and lockfile, so its dependency tree (winit, glow and the rest) stays out of the signer's gate. `just apps console-check` runs its fmt, clippy, tests, `cargo audit` and the 7-day dependency age; CI does not run it.

`src/log.rs` mirrors the signer's audit `Entry` (`src/audit.rs`) and `limits` (`src/engine.rs`). Change them together: a test parses lines written by the signer (`tests/fixtures/signer-audit.jsonl`), so a change to the signer's `Entry` fails it rather than silently breaking the chain check.
