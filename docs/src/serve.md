---
description: "serve: the HTTP routes and MCP tools, bearer tokens and allowed hosts, graceful shutdown, adding it to Claude Code, the llms.txt resources and the agent skill, and sa-forge-signer-mcp for MCP clients that speak only stdio"
---

# Serving over HTTP and MCP

`sa-forge-signer-serve` runs the signer as a long-lived service (called `serve` in this book, after its `[serve]` config table). It is its own binary, installed beside `sa-forge-signer`, and takes the same `--config`; it replaces the `sa-forge-signer serve` subcommand, which no longer exists. As a service, each signature skips process start-up and keys stay loaded in one isolated place. It listens on `[serve] listen` (default `127.0.0.1:8790`):

- `POST /v1/check` and `POST /v1/sign`: body `{"payload": <payload object>, "key": "optional", "intent": "optional"}`; the reply is the same [JSON report](check-and-sign.md#the-report) as the CLI. `4xx` means a request or auth problem, `500` that the signer could not finish (see `error`).
- `POST /v1/halt`: body `{"reason": "...", "key": "optional"}`; halts that key, or every key the token lists, until a person runs `resume` at the signer's terminal ([Halting](check-and-sign.md#halting)). The reply is `{"halted": [<keys>]}`. A key the token does not list is `403`; a missing or empty `reason` (or one over 500 characters) is `400`. There is no resume route.
- `POST /mcp`: MCP, protocol revision `2026-07-28` only (JSON responses, no streaming; [MCP conformance](dev/mcp-conformance.md) lists the rules it follows and what a client of an earlier revision sees) with tools `check`, `sign`, `key_show` and `halt` (the same arguments as `/v1/halt`), and no resume. `key_show` returns the key's `pubkey`, `class`, `approval`, `profile` and `balance_lamports`, `halted` (`null`, or `since`, `by` and `reason`) and `policy` (`digest`, `approval_required`, `approved`; see [Policy digest](check-and-sign.md#policy-digest)), so an agent can tell whether the key will sign before it builds a payload. Neither a halt nor an approval can be lifted or given through `serve`. It also serves four read-only [resources](#resources-and-the-skill) that tell an agent how to use the tools.
- `GET /health`: no auth, for health checks.

Every other route needs `Authorization: Bearer <token>`. Create a token with `sa-forge-signer token new <name> --out <file>`: it writes the token to a new 0600 file and prints only its sha256 for `[[serve.tokens]]`, which also lists the keys that token may use. Host and Origin headers must name an entry in `allowed_hosts`. `confirm` keys prompt on the service's own terminal; without one they refuse.

The smallest `[serve]` table is the `[serve]` line and one `[[serve.tokens]]` entry: `listen` defaults to `127.0.0.1:8790` and `allowed_hosts` to `127.0.0.1` and `localhost`, so set them only to listen elsewhere (in Docker, `listen = "0.0.0.0:8790"`). `serve` refuses to start without a token, and every key a token lists must be in the config. A `wallet` key can be on a token only if it sets `allow_unattended = true`, or the config does not load; to sign through `serve` without a person at its terminal it also needs `approval = "auto"`. Both are for throwaway testnet keys only ([Getting started](getting-started.md#testnet-the-agent-holds-a-throwaway-wallet)).

On SIGTERM or SIGINT the service stops taking requests and lets running ones finish, for up to 220 s, then exits. A sign waits up to 150 s for an outcome, and the poll that crosses that deadline can make four more RPC calls of up to 15 s each. Give it a stop grace period of at least 230 s (for example `docker stop -t 230`, or `TimeoutStopSec=230`). That covers the usual case, not every case: the checks before sending also call the RPC, and a stalled RPC can stretch them. A sign cut off by the stop has already recorded its intent; a client that loses the connection during `sign` must treat it as `unknown` and [reconcile](check-and-sign.md#retrying-safely) before retrying.

MCP clients pass the payload object from the forge-mcp `build_*` result unchanged, and call `check` before `sign`. In a 15-payload test every payload arrived byte-exact, but the model re-types each one, so each costs its tokens twice and any partial-signer secret appears twice in the transcript.

Add it to Claude Code with `claude mcp add --transport http sa-forge-signer http://127.0.0.1:8790/mcp --header "Authorization: Bearer $(cat <file>)"`. The client must speak MCP `2026-07-28`: an earlier one is refused at its `initialize` with an error naming that revision.

## Resources and the skill

The MCP route lists four resources (`resources/list`, `resources/read`), all static text written for the agent, with nothing from the config, the keys or the audit log:

| Resource | Holds |
|---|---|
| `sa-forge-signer://llms.txt` | The index: what the signer is, and a line on each tool and resource. The server's instructions tell a client to start here |
| `sa-forge-signer://llms-full.txt` | The guide and the tool reference in one file |
| `skill://sa-forge-signer/SKILL.md` | The guide: sending an action, acting on each outcome, reconciling an `unknown` one, and when to halt |
| `skill://sa-forge-signer/references/tools.md` | Each tool with its arguments, and every field of the report |

The last two are the skill in [`skills/sa-forge-signer/`](../../skills/sa-forge-signer/SKILL.md), built into the binaries, so what the service serves and what gets installed are the same files. To install it as a Claude skill on the agent's side:

```sh
sa-forge-signer-mcp skill print                    # the skill file, on standard output
sa-forge-signer-mcp skill install                  # ~/.claude/skills/sa-forge-signer/
sa-forge-signer-mcp skill install --project .      # ./.claude/skills/sa-forge-signer/
```

`skill install` writes `SKILL.md` and `references/tools.md`, replaces only a copy it wrote earlier, and refuses to touch a `SKILL.md` of that name it did not write. Neither command needs the token or a running service.

## MCP over stdio

`sa-forge-signer-mcp` is for an MCP client that starts its servers as child processes and talks to them on standard input and output. It does not replace `sa-forge-signer-serve`: the service must be running as its own process, and a stdio setup is the two together. It is a third binary from the same crate and a client of that service, not a second signer: it forwards each JSON-RPC message to the service's `/mcp` route with the bearer token, adding the HTTP headers that revision requires from the message itself, and writes the reply back, so the tools, their arguments and every check are the ones above.

| Process | Started by | Runs as | Holds |
|---|---|---|---|
| `sa-forge-signer-serve` | you: a terminal, a service manager or Docker | the signer's OS user | the keys and the config |
| `sa-forge-signer-mcp` | the agent's MCP client, as a child process | the agent's OS user | the service's URL and a token |

A client that supports MCP over HTTP, as Claude Code does, does not need it: point the client at the service's `/mcp` route as above.

```sh
sa-forge-signer-mcp --token-file ~/.config/sa-forge-signer/agent.token      # --url defaults to http://127.0.0.1:8790
claude mcp add sa-forge-signer -- sa-forge-signer-mcp --token-file ~/.config/sa-forge-signer/agent.token
```

- It reads no config and no key, and cannot sign on its own: the MCP client starts it as the agent's OS user, which the [isolation rule](setup.md#isolation) keeps away from both. It holds what the HTTP setup already gives the agent, the service's URL and a token.
- `--url` must be `https://`, or `http://` only to a loopback host (`127.0.0.1`, `::1`, `localhost`), so the token never crosses a network in clear. `--token-file` is required; the file's content is the token, with surrounding whitespace removed.
- Without a running `sa-forge-signer-serve` every request fails. A request the service cannot be reached for, or answers without a JSON-RPC reply (a wrong token is HTTP `401`), comes back as a JSON-RPC error with code `-32603` naming the cause. For a `sign` that means the outcome is `unknown`: [reconcile](check-and-sign.md#retrying-safely) before retrying.
- It waits up to 240 s for each reply, longer than the service's own 220 s drain. `confirm` keys still prompt on the service's terminal, never here.
- The release archive carries it beside the other two, and the [install script](install.md#install-script) installs it with them; `cargo install` from a tag builds all three.

