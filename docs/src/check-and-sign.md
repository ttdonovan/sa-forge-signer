---
description: "The CLI: check and sign, the JSON report, outcomes and exit codes, expired blockhashes, retrying safely after unknown, and partial signers"
---

# Checking and signing

```sh
# Generate a key; prints only the pubkey and a config snippet
sa-forge-signer key new alice-session --class session

# Configure it (see Configuration), then grant it on the profile with
# forge-mcp build_add_profile_key, signed by the profile authority.

sa-forge-signer check payload.json                  # checks + simulation, never signs
sa-forge-signer sign --intent "scan round" payload.json
sa-forge-signer key show alice-session
sa-forge-signer key fund alice-session --zink 0.05 --from alice-wallet --out fund.json
sa-forge-signer key destroy alice-session
sa-forge-signer audit verify
```

The config lives at `~/.config/sa-forge-signer/config.toml` unless `--config` or `SA_FORGE_SIGNER_CONFIG` says otherwise (see [Configuration](configuration.md)); a key's whole life is in [Key lifecycle](key-lifecycle.md). The key is chosen by `--key`, or by matching the payload's fee payer.

## The report

`check` and `sign` print one JSON report (`outcome`, `signature`, `slot`, `explorer`, `detail`, `failed_check`, `program_error`, `error`, `checks`, `compute_units`, `fee`, `balance_change`, `summary`, `logs`). `checks` lists the checks that passed; on a refusal `failed_check` names the one that failed. `program_error` is the program's own message from the logs, on a failed simulation or a transaction that landed with an error. For a landed transaction, `balance_change` and `fee` come from the transaction's own metadata, so parallel transactions from the same key do not skew them; for `check` they are simulated. The payload's `summary` comes from the builder and is not verified.

## Outcomes

| Outcome | Exit | Meaning | Retry? |
|---|---|---|---|
| `ok` | 0 | `check` passed | - |
| `confirmed` | 0 | landed | no |
| `refused` | 10 | a check failed | fix the payload; after `failed_check` `blockhash`, nothing was sent and a retry is safe |
| `simulation_failed` | 11 | the program would reject it | after fixing the cause |
| `failed` | 12 | landed with a program error | after fixing the cause |
| `unknown` | 14 | sent but not proven landed: the blockhash expired with no status found, 150 s passed, or the RPC kept failing | only after [reconciling](#retrying-safely) |

Before anything is sent, the `blockhash` check compares the transaction's last valid height with the finalized height: a blockhash already past it can never land, whichever node reports the height (a lagging RPC node can hand out an expired blockhash). It is re-signed and simulated once with a fresh one; if that is expired too, the sign is refused and nothing is sent.

The same instructions signed with the same blockhash give the same signature, so a payload sent twice within one blockhash would resend the first transaction, and the second sign would report the first one's outcome as its own. Before the intent is recorded, the `duplicate` check looks up the transaction's signature among the key's audit entries from the last 24 h. On a match it is re-signed and simulated once with a fresh blockhash; if the RPC returns the same blockhash again, the sign is refused with `failed_check` `blockhash`, nothing is sent, and a retry is safe.

Exit code 13 (`expired`) is retired. An RPC cannot prove that a transaction never landed: behind a load balancer, the finalized height and the status lookup can come from different nodes, and a node can lack history. So an expired blockhash with no status found is reported as `unknown`, with that in `detail`.

## Retrying safely

After `unknown`, an HTTP `500`, a dropped connection or a restarted service, the transaction may still have landed. Before building the action again:

1. Look up the reported signature (or the `signature` of the key's last `sign-intent` entry in the audit log).
2. Re-read the game state the action was meant to change (the fleet's state, the cargo, the balance).
3. Retry only if the action did not happen.

The per-key lock and the intent entry stop concurrent signs from racing the limits; they do not stop a caller from sending the same action twice with a new blockhash.

## Partial signers

Payloads that carry partial signers hold private keys for new accounts; `sign` deletes such a payload file once it has been sent, and never logs them.
