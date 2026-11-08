---
description: "config init and config show, and the annotated example config: cluster, RPC, key and state directories, per-key class, approval, limits and transfer destinations, and serve tokens"
---

# Configuration

The config lives at `~/.config/sa-forge-signer/config.toml` unless `--config` or `SA_FORGE_SIGNER_CONFIG` says otherwise. `sa-forge-signer config init` writes the example below there (mode 0600, in a 0700 directory it creates if needed) and refuses to overwrite an existing file. It has no keys yet: add the block `key new` prints for each key, and `[serve]` if you run `serve`.

`sa-forge-signer config show` prints the config as the signer resolves it: the paths, the RPC, and the instruction table if one is set, each key with its effective class, approval, limits and whether its key file exists, defaults filled in, then `[serve]` with each token's name and keys. It reads no key material, prints no token hashes, and calls no RPC.

## Settings

Only `class` and `pubkey` on each key, and a token for `serve`, have to be set; everything else has a default.

| Setting | Default | Set it when |
|---|---|---|
| `cluster` | `"zink-testnet"` | never: it is the only cluster |
| `rpc_url` | the cluster's public RPC (`https://rpc1.z.ink`) | you use another RPC you trust ([Security](security.md)) |
| `explorer_url` | the cluster's explorer (`https://explorer.z.ink`) | you prefer another block explorer. Used only for links: the `explorer` field of a `sign` report and the [console](apps/console.md)'s links; the signer never calls it |
| `key_dir` | `keys`, beside the config | keys live elsewhere (in Docker, `/run/keys`) |
| `state_dir` | `state`, beside the config | the audit log lives elsewhere (in Docker, `/var/lib/sa-forge-signer`) |
| `instruction_table` | none | you want the [instruction table](apps/idl-table.md)'s findings: `{ path = "...", sha256 = "...", mode = "report" }`, the path relative to the config. The signer refuses to start if the file's sha256 differs or it is for another cluster. `report` adds `findings` to each report and audit entry and never refuses; an enforcing mode comes later |
| `[keys.<name>] class` | required | `"session"` for a SAGE-scoped profile key, `"wallet"` for the profile authority |
| `pubkey` | required | the pubkey `key new` or `key import` printed |
| `profile` | none | you want `key show` and the MCP `key_show` tool to report the key's profile, or you configure an `instruction_table`: its `profile` finding holds the key to this profile |
| `approval` | `"auto"` for a session key, `"confirm"` for a wallet key | a key should never sign (`"deny"`), or a testnet wallet should sign unattended (`"auto"`, with `allow_unattended`) |
| `allow_unattended` | `false` | a throwaway testnet wallet key must be `"auto"` or on a serve token |
| `rate_limit_per_minute` | `30` | the agent signs faster than that |
| `daily_lamport_cap` | `50000000` (0.05 ZINK) | the key spends more in 24 h (fees and transfers it signs) |
| `halt_after_unknown` | `3` | the key should [halt](check-and-sign.md#halting) after more or fewer consecutive `unknown` outcomes; `0` never halts it |
| `max_compute_units` | `1400000` (the runtime's maximum) | the key's payloads should be held to a lower `SetComputeUnitLimit` |
| `max_compute_unit_price` | `0` (no priority fee) | the key may pay a priority fee: the highest `SetComputeUnitPrice`, in micro-lamports per compute unit |
| `transfer_max_lamports` | none (only the daily cap) | the key's System transfers, summed over one transaction, should stay under a smaller amount |
| `transfer_to` | none | the key funds other accounts with ZINK, such as a wallet funding its session key or vault |
| `extra_programs` | none | the key must call a program outside the built-in list ([How it works](how-it-works.md)); add only programs you trust |
| `[serve] listen` | `"127.0.0.1:8790"` | `serve` should listen elsewhere (in Docker, `"0.0.0.0:8790"`) |
| `allowed_hosts` | `["127.0.0.1", "localhost"]` | clients reach `serve` under another host name |
| `[[serve.tokens]]` | none: `serve` will not start | always, for `serve`: `name`, the `sha256` `token new` printed, and the `keys` it may use |

The example, as `config init` writes it:

```toml
{{#include ../../config.example.toml}}
```
