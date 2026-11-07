---
description: "serve: the HTTP routes and MCP tools, bearer tokens and allowed hosts, graceful shutdown, and adding it to Claude Code"
---

# Serving over HTTP and MCP

`sa-forge-signer serve` runs the signer as a long-lived service, so each signature skips process start-up and keys stay loaded in one isolated place. It listens on `[serve] listen` (default `127.0.0.1:8790`):

- `POST /v1/check` and `POST /v1/sign`: body `{"payload": <payload object>, "key": "optional", "intent": "optional"}`; the reply is the same [JSON report](check-and-sign.md#the-report) as the CLI. `4xx` means a request or auth problem, `500` that the signer could not finish (see `error`).
- `POST /mcp`: MCP (JSON responses, no streaming) with tools `check`, `sign` and `key_show`.
- `GET /health`: no auth, for health checks.

Every other route needs `Authorization: Bearer <token>`. Create a token with `sa-forge-signer token new <name> --out <file>`: it writes the token to a new 0600 file and prints only its sha256 for `[[serve.tokens]]`, which also lists the keys that token may use. Host and Origin headers must name an entry in `allowed_hosts`. `confirm` keys prompt on the service's own terminal; without one they refuse.

On SIGTERM or SIGINT the service stops taking requests and lets running ones finish, for up to 220 s, then exits. A sign waits up to 150 s for an outcome, and the poll that crosses that deadline can make four more RPC calls of up to 15 s each. Give it a stop grace period of at least 230 s (for example `docker stop -t 230`, or `TimeoutStopSec=230`). That covers the usual case, not every case: the checks before sending also call the RPC, and a stalled RPC can stretch them. A sign cut off by the stop has already recorded its intent; a client that loses the connection during `sign` must treat it as `unknown` and [reconcile](check-and-sign.md#retrying-safely) before retrying.

MCP clients pass the payload object from the forge-mcp `build_*` result unchanged, and call `check` before `sign`. In a 15-payload test every payload arrived byte-exact, but the model re-types each one, so each costs its tokens twice and any partial-signer secret appears twice in the transcript.

Add it to Claude Code with `claude mcp add --transport http signer http://127.0.0.1:8790/mcp --header "Authorization: Bearer $(cat <file>)"`.
