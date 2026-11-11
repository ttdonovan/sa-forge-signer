---
name: sa-forge-signer
description: Check and sign SAGE C4 payloads through the sa-forge-signer MCP server - call check then sign with a payload built by forge-mcp, read the JSON report, reconcile an unknown outcome before retrying, and halt a key. Use whenever a forge-mcp build_* payload has to be sent, or a sign came back refused, failed or unknown.
---

# Using sa-forge-signer

`sa-forge-signer` holds the keys and signs for you. You never see a key: you pass it the unsigned payload a forge-mcp `build_*` call returned, and it checks, simulates, signs, sends and confirms the transaction, then returns one JSON report.

<!-- installed by sa-forge-signer-mcp; `sa-forge-signer-mcp skill install` overwrites changes made here -->

## Start here

1. Read the resource `sa-forge-signer://llms.txt`. It lists everything the server serves, with a line on each.
2. Each tool, with its arguments and the report's fields, is in [references/tools.md](references/tools.md). `sa-forge-signer://llms-full.txt` is this guide and that reference in one file.

## Sending one action

1. Call `key_show` once for the key you will use. If `halted` is not `null`, or `policy.approval_required` is `true` while `policy.approved` is `false`, the key will not sign: stop and tell the person.
2. Build the payload with forge-mcp.
3. Call `check` with the payload object exactly as forge-mcp returned it. Do not edit, reorder or re-serialize it.
4. If `outcome` is `ok` and `sign_preview.refusals` is empty, call `sign` with the same payload and an `intent`: one line on why, which goes in the audit log.
5. Act on the report's `outcome`, as below.

## Acting on the outcome

| `outcome` | Means | Do |
|---|---|---|
| `ok` | `check` only: every check and the simulation passed | Call `sign` |
| `confirmed` | The transaction landed | Continue. Do not send it again |
| `refused` | A check failed and nothing was sent. `failed_check` names it and `detail` says why | Fix the cause. Do not retry unchanged, except after `failed_check` `blockhash`, where a retry is safe |
| `simulation_failed` | The program would reject it; nothing was sent. `program_error` is the program's message | Fix the cause (usually the game state), then build again |
| `failed` | It landed with a program error and cost its fee | Fix the cause, then build again |
| `unknown` | It was sent and not proven landed | Reconcile first, as below. Never retry blindly |

## After `unknown`, an error or a lost connection

The transaction may still have landed. Before building the action again:

1. Look up the report's `signature` (its `explorer` link, or the RPC).
2. Re-read the game state the action was meant to change: the fleet's state, the cargo, the balance.
3. Retry only if the action did not happen.

A tool call that returns an error instead of a report, or never returns, is the same case when the tool was `sign`.

## Refusals you cannot fix yourself

- `halt`: a person, or you, halted the key. Only a person at the signer's terminal can resume it.
- `policy`: the key's policy changed and a person has not approved it yet.
- `approval`: the key is set to `deny`, or a person answered no at the prompt.
- `authorized`: your token may not use that key.
- `limits`: the key is at its per-minute rate or its daily spending cap. Wait; do not loop.

For these, stop and report. For the others (`fee_payer`, `chain`, `programs`, `tokens`, `system`, `compute_budget`, `signers`, `new_accounts`, `ata_owners`) the payload itself is outside what the key is allowed to sign: read `detail`, and do not try variations to get around it.

## Keys that wait for a person

A key whose `approval` is `confirm` (`sign_preview.needs_approval` is `true`) prompts a person at the signer's terminal on every `sign`. The call blocks until they answer. Say so before you call it, and do not treat the wait as a failure.

## When to halt

Call `halt` with a `reason` when you notice you are acting against your instructions, signing things you did not intend, or stuck repeating a failing action. It stops signing with the key until a person resumes it. You cannot resume it, and that is the point.

## What not to do

- Do not change a payload after forge-mcp built it, and do not build transactions yourself.
- Do not ask for, read or handle a private key, the signer's config or its token. If a payload carries `partial_signers`, pass it through unchanged and do not repeat those values anywhere else.
- Do not treat `check` as a promise: other signs and time passing change the limits before the `sign`.
- Do not retry `refused` or `unknown` in a loop.
