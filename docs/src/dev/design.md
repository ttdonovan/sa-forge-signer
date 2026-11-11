---
description: "Design decisions and their reasons: one crate with three binaries by role, why the stdio MCP binary is a client of the service and not a second signer, why the service uses tiny_http and a hand-written MCP handler instead of rmcp, the built-in agent guides, and what would reopen each choice"
---

# Design decisions

Choices about the signer's shape that are not obvious from the code, each with its reason, what it costs, and what would reopen it. What the signer checks is in [How it works](../how-it-works.md), the threats it answers in [Security](../security.md), and work not yet built in [Hardening proposals](hardening.md) and the [Plan](plan.md). Add an entry here when a change settles a question someone is likely to ask again.

## One crate, three binaries by role

**Decision (2026-11-11).** The crate is a library and three binaries: `sa-forge-signer` (the operator's command line and one-shot `check` and `sign`), `sa-forge-signer-serve` (the long-running service) and `sa-forge-signer-mcp` (MCP on standard input and output). The `serve` subcommand was removed when the service became its own binary.

**Why.** The three are run by different parties in different places: a person at a terminal, a service manager or container, and an agent's MCP client. A binary that does one of those is easier to install, start and reason about than a subcommand of another. One crate keeps one version, one changelog and one release.

**Cost.** A release carries more than one binary, and the Docker image's entrypoint is the service, so its command line needs `--entrypoint sa-forge-signer` ([Running in Docker](../docker.md)).

## The library is not a public API

**Decision (2026-11-11).** `src/lib.rs` exists so the binaries share one implementation. Its modules are private; the only public items are each binary's `main`.

**Why.** The interface callers depend on is the command line, the routes and the file formats ([Contributing](contributing.md)). A public Rust API would be a second interface to keep stable, for no caller that exists.

**Reopen when** another crate has a real need to link the signer's logic. The apps mirror a little of it today and are kept in step by tests ([Building an app](apps.md)).

## The stdio MCP binary is a client of the service, not a second signer

**Decision (2026-11-11).** `sa-forge-signer-mcp` forwards each MCP message to a running service's `/mcp` route with a bearer token. It reads no key and no config and cannot sign on its own.

**Why.** An MCP client starts a stdio server as a child process, so it runs as the agent's OS user. The [isolation rule](../setup.md#isolation) is that this user cannot read the keys or the config. A stdio server with the engine inside it would need both, and a `confirm` prompt would land on the agent's own terminal. As a client it holds only what the HTTP setup already gives the agent: the service's URL and a token.

**Cost.** stdio needs the service running; there is no single-process MCP setup.

**Reopen when** never for keys of value. An in-process mode could only suit the setup where the agent already holds a throwaway testnet wallet and runs the command line as itself, and it would have to refuse `confirm` keys.

## The service is synchronous: `tiny_http` and a hand-written MCP handler

**Decision (2026-11-11, confirming how it was first built).** The service is a thread-per-request HTTP server on `tiny_http`, and `src/serve/mcp.rs` handles MCP's JSON-RPC by hand: `server/discover`, the tools and the resources, with JSON replies and no streaming. It does not use `rmcp`, the Rust MCP SDK.

`rmcp` does provide both a stdio and a streamable HTTP transport. That does not change the entry above: whichever library speaks the protocol, a stdio server still runs as the agent's user, so the stdio binary would still be a client of the service.

**Why.**

- The engine blocks from end to end: RPC calls, file locks, a terminal prompt, and a wait of up to 150 s for an outcome. A thread per request fits that as it is. Under an async server every tool call would have to be moved onto a blocking thread.
- The service also has plain HTTP routes (`/v1/check`, `/v1/sign`, `/v1/halt`, `/health`) that an MCP SDK does not provide, so an HTTP framework would be needed beside it.
- No async runtime keeps the dependency tree small in the one process that holds the keys. `rmcp`'s HTTP server brings `tokio` and an HTTP stack with it; how many crates that adds has not been measured.
- Each new dependency is subject to the 7-day age rule and to `cargo audit` in the gate.

**Cost.** MCP features have to be written by hand as they are needed, and tool schemas are written out rather than derived from types. The handler does not stream, so a client sees nothing between calling `sign` and its report.

**Reopen when** a feature is wanted that the hand-written handler cannot reasonably grow, most likely progress messages during a long `sign`. Moving the service to `rmcp` then means an async HTTP framework for the other routes, the token, Host and Origin checks as middleware, and the shutdown drain rebuilt. The MCP code is in one file so that change stays contained. What that file implements of the specification, and what it does not, is in [MCP conformance](mcp-conformance.md).

## One MCP revision, the latest

**Decision (2026-11-11).** The signer speaks MCP `2026-07-28` and no earlier revision.

**Why.** That revision is stateless: no handshake, no session, each request complete in itself. That is how the hand-written handler already worked, and it removes the parts a second revision would have forced it to keep (`initialize`, `ping`, version negotiation, two sets of error codes). Supporting both eras means carrying both behaviours in a handler with no SDK behind it.

**Cost.** A client that speaks only an earlier revision cannot connect; it gets an error naming `2026-07-28` ([MCP conformance](mcp-conformance.md)).

**Reopen when** a client that matters cannot be upgraded. The specification allows one server to serve both eras on the same endpoint.

## Agent guidance ships in the binary

**Decision (2026-11-11).** The service serves `llms.txt`, `llms-full.txt` and a skill as MCP resources, and `sa-forge-signer-mcp skill install` writes the same skill to disk ([Resources and the skill](../serve.md#resources-and-the-skill)). The text is `skills/sa-forge-signer/`, built in at compile time.

**Why.** Tool descriptions alone do not tell an agent what to do after `unknown`, which refusals it cannot fix, or when to halt. Building the files in means what is served, what is installed and what the code does are one version.

**Cost.** The guides are static: they do not list the keys a token may use (`key_show` answers that), and a change to the report or the tools has to update `skills/sa-forge-signer/` in the same change. A test fails if the guides and the tool list name different tools.

## Apps stay out of the signer's gate

**Decision (standing rule; its workflow added 2026-11-10).** Each app under `apps/` is its own crate, workspace and lockfile, outside `just check`, the `ci` workflow, the release binaries and the Docker image. A separate `apps` workflow runs the grant page's and the IDL table generator's own gates ([Building an app](apps.md)).

**Why.** The apps pull in large dependency trees (a UI framework, a wasm toolchain) that should neither slow nor widen the gate of the binary that holds keys.

**Cost.** The console's gate runs only locally, and an app can break without failing the signer's required checks.

**Reopen when** the console's build requirements on a CI runner are known, to add it to the `apps` workflow.
