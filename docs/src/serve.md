---
description: "serve: the HTTP routes and MCP tools, bearer tokens and allowed hosts, graceful shutdown, and adding it to Claude Code"
---

# Serving over HTTP and MCP

`sa-forge-signer-serve` runs the signer as a long-lived service (called `serve` in this book, after its `[serve]` config table). It is its own binary, installed beside `sa-forge-signer`, and takes the same `--config`; it replaces the `sa-forge-signer serve` subcommand, which no longer exists. As a service, each signature skips process start-up and keys stay loaded in one isolated place. It listens on `[serve] listen` (default `127.0.0.1:8790`):

- `POST /v1/check` and `POST /v1/sign`: body `{"payload": <payload object>, "key": "optional", "intent": "optional"}`; the reply is the same [JSON report](check-and-sign.md#the-report) as the CLI. `4xx` means a request or auth problem, `500` that the signer could not finish (see `error`).
- `POST /v1/halt`: body `{"reason": "...", "key": "optional"}`; halts that key, or every key the token lists, until a person runs `resume` at the signer's terminal ([Halting](check-and-sign.md#halting)). The reply is `{"halted": [<keys>]}`. A key the token does not list is `403`; a missing or empty `reason` (or one over 500 characters) is `400`. There is no resume route.
- `POST /mcp`: MCP (JSON responses, no streaming) with tools `check`, `sign`, `key_show` and `halt` (the same arguments as `/v1/halt`), and no resume. `key_show` returns the key's `pubkey`, `class`, `approval`, `profile` and `balance_lamports`, `halted` (`null`, or `since`, `by` and `reason`) and `policy` (`digest`, `approval_required`, `approved`; see [Policy digest](check-and-sign.md#policy-digest)), so an agent can tell whether the key will sign before it builds a payload. Neither a halt nor an approval can be lifted or given through `serve`.
- `GET /health`: no auth, for health checks.

Every other route needs `Authorization: Bearer <token>`. Create a token with `sa-forge-signer token new <name> --out <file>`: it writes the token to a new 0600 file and prints only its sha256 for `[[serve.tokens]]`, which also lists the keys that token may use. Host and Origin headers must name an entry in `allowed_hosts`. `confirm` keys prompt on the service's own terminal; without one they refuse.

The smallest `[serve]` table is the `[serve]` line and one `[[serve.tokens]]` entry: `listen` defaults to `127.0.0.1:8790` and `allowed_hosts` to `127.0.0.1` and `localhost`, so set them only to listen elsewhere (in Docker, `listen = "0.0.0.0:8790"`). `serve` refuses to start without a token, and every key a token lists must be in the config. A `wallet` key can be on a token only if it sets `allow_unattended = true`, or the config does not load; to sign through `serve` without a person at its terminal it also needs `approval = "auto"`. Both are for throwaway testnet keys only ([Getting started](getting-started.md#testnet-the-agent-holds-a-throwaway-wallet)).

On SIGTERM or SIGINT the service stops taking requests and lets running ones finish, for up to 220 s, then exits. A sign waits up to 150 s for an outcome, and the poll that crosses that deadline can make four more RPC calls of up to 15 s each. Give it a stop grace period of at least 230 s (for example `docker stop -t 230`, or `TimeoutStopSec=230`). That covers the usual case, not every case: the checks before sending also call the RPC, and a stalled RPC can stretch them. A sign cut off by the stop has already recorded its intent; a client that loses the connection during `sign` must treat it as `unknown` and [reconcile](check-and-sign.md#retrying-safely) before retrying.

MCP clients pass the payload object from the forge-mcp `build_*` result unchanged, and call `check` before `sign`. In a 15-payload test every payload arrived byte-exact, but the model re-types each one, so each costs its tokens twice and any partial-signer secret appears twice in the transcript.

Add it to Claude Code with `claude mcp add --transport http sa-forge-signer http://127.0.0.1:8790/mcp --header "Authorization: Bearer $(cat <file>)"`.
