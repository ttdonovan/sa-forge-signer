---
description: "sa-forge-signer-console: a read-only desktop view of serve health, the audit log and per-key limits, run locally or against a signer in Docker. Early spike."
---

# Console

`sa-forge-signer-console` is a desktop window, built with [GPUI Kit](https://gpui-kit.com), for watching a signer. It reads the signer's audit log and config and polls `serve`'s `/health`; it never writes, locks, signs or approves anything. It is an early spike in `apps/console/`. It always uses its own dark theme, whatever the system appearance, with the [Tomorrow](https://fonts.google.com/specimen/Tomorrow) font for headings and labels (bundled in `apps/console/assets/fonts/` under the SIL Open Font License 1.1, `OFL.txt` beside it). Building it needs Rust 1.95 or later; on macOS, macOS 15 or later and the Xcode Command Line Tools (`xcode-select --install`).

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

- **Status bar:** `serve` up or down, the audit log's hash chain (intact, or the first line that breaks it), how many sends are unresolved, and how long ago the last entry was written.
- **Keys:** each key's class, approval and pubkey (with a copy button and an explorer link), its last activity, and two bars: signs in the last minute against its rate limit, and ZINK spent or reserved in 24 h against its daily cap, as the signer computes them (see [How it works](../how-it-works.md)). A bar turns amber at 80% and red at the limit. Click a key to filter the log by it; the key column is hidden while one key is shown.
- **Audit log:** newest first, one row per signature: the `sign-intent` the signer writes before sending is merged into the `sign` entry with its outcome. An intent with no outcome stays as its own row, marked unresolved, as is an `unknown` or `expired` outcome; reconcile them before retrying (see [Retrying safely](../check-and-sign.md#retrying-safely)). Each row shows its balance change in ZINK, the intent the agent gave, then the builder's summary with addresses shortened (`GwDE…dFvg`). Rows that arrive while the console is open are highlighted for a few seconds. The filter box matches intent, summary, key, signature and outcome. "raw entries" shows every audit entry as the hash chain stores it, with its command.
- **Detail:** click a row for a side panel: what was signed (intent, summary, programs), the result (times in UTC, signature, slot, balance change and the reserved spend in ZINK), and the hashes that tie it into the chain. Signatures, programs and slots have a copy button and link to the block explorer in the config's `explorer_url` (by default `https://explorer.z.ink`; see [Configuration](../configuration.md#settings)).

**Keys:** with the log focused, ↑ and ↓ move through the rows; `/` jumps to the filter box, and Esc leaves it or closes the detail panel. Cmd-Q quits.

It checks `audit.jsonl` about twice a second and, when its size or modification time has changed, reads only the lines appended since its last read, continuing the hash chain from there; a last line still being written is skipped. If the log shrank or the last line it read has changed, it reads the whole log again. An edit above that line would not be seen this way, so once a minute, if the log changed, it reads and checks the whole log. The config supplies key classes, approval and limits; a key that appears only in the log is shown with the default limits. The config holds no secrets, only token hashes.

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

The crate has its own workspace and lockfile, so its dependency tree (GPUI and the rest) stays out of the signer's gate. `just apps console-check` runs its fmt, clippy, tests, `cargo audit` and the 7-day dependency age; CI does not run it.

Changing the UI: GPUI Kit's guides and docs, and how to read them, are in [Building an app](../dev/apps.md); its Claude skills can be installed with `just apps gpui-kit-skills` (optional; see [GPUI Kit skills](../appendix/gpui-kit-skills.md)).

`src/log.rs` mirrors the signer's audit `Entry` (`src/audit.rs`) and `usage` (`src/facts.rs`). Change them together: a test parses lines written by the signer (`tests/fixtures/signer-audit.jsonl`), so a change to the signer's `Entry` fails it rather than silently breaking the chain check.
