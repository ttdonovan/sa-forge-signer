---
description: "MCP conformance: the specification the signer follows and where to read it, the revisions it speaks, what the service's /mcp route and the stdio binary implement rule by rule, the known gaps, and what the 2026-07-28 revision would change"
---

# MCP conformance

The signer speaks the [Model Context Protocol](https://modelcontextprotocol.io/specification/) with a handler written by hand (`src/serve/mcp.rs`), not an SDK ([Design decisions](design.md#the-service-is-synchronous-tiny_http-and-a-hand-written-mcp-handler) says why). With no SDK to keep it honest, this page is the record of what it implements, checked against the specification's pages on 2026-11-11.

The specification to read is the one at `modelcontextprotocol.io`, which is built from the `modelcontextprotocol/specification` repository. Other sites reproduce it; when they differ, that one is right. Each revision has its own tree, such as `https://modelcontextprotocol.io/specification/2025-11-25/`.

## Revisions

| Revision | Status here |
|---|---|
| `2026-07-28` (the latest) | Not implemented. See [The 2026-07-28 revision](#the-2026-07-28-revision) |
| `2025-11-25` | Implemented, within the scope below. The version the service answers with when a client asks for one it does not know |
| `2025-06-18` | Implemented, within the scope below |
| `2025-03-26` | Negotiated, with one gap: batches ([Known gaps](#known-gaps)) |

`initialize` answers with the client's requested version when it is one of the three, and otherwise with `2025-11-25`, as the lifecycle rules require.

## Scope

The service is an MCP server with two features: tools and resources. It offers no prompts, logging, completions, sampling, roots, elicitation or tasks, and it never sends a request or a notification to the client. Its lists are short and fixed, so they are never paginated and never change (`listChanged` is `false`).

| Method | Answer |
|---|---|
| `initialize` | Protocol version, `capabilities` (`tools`, `resources`), `serverInfo` and `instructions` |
| `notifications/initialized`, and any other notification | Accepted and ignored. The handler keeps no state, so it does not require this before other requests |
| `ping` | An empty result |
| `tools/list` | `check`, `sign`, `key_show` and `halt`, each with an `inputSchema` |
| `tools/call` | One text content block and `isError` ([Serving over HTTP and MCP](../serve.md)) |
| `resources/list`, `resources/read` | Four static texts ([Resources and the skill](../serve.md#resources-and-the-skill)) |
| anything else | JSON-RPC error `-32601`, method not found |

## The HTTP transport

`POST /mcp` on `sa-forge-signer-serve` follows the Streamable HTTP transport, in its plain form: one JSON object per reply, no event stream.

| Rule (2025-11-25) | The service |
|---|---|
| A request may be answered with `Content-Type: application/json` or an event stream | Always `application/json` |
| An accepted notification or response gets `202 Accepted` with no body | Yes |
| `GET` on the endpoint returns an event stream or `405 Method Not Allowed` | `405`: there is no stream |
| The `Origin` header must be validated, and an invalid one answered with `403 Forbidden` | Yes: `Origin` and `Host` must name an entry in `allowed_hosts`, else `403` |
| Sessions (`MCP-Session-Id`) are optional | None: every request stands alone |
| The body is a single JSON-RPC message | A body that is not JSON is `400` with error `-32700`; an array is `400` with `-32600` |
| A request with an invalid or unsupported `MCP-Protocol-Version` header is answered with `400 Bad Request` | **Not done.** The header is not read ([Known gaps](#known-gaps)) |

Authentication is outside the specification's own scheme: the service takes a bearer token from its config, not OAuth. A request without a known token is `401` before the body is read.

## Tools and resources

| Rule (2025-11-25) | The service |
|---|---|
| A tool that ran and failed reports it in its result with `isError: true`; a protocol problem is a JSON-RPC error | A report whose `outcome` is not `ok` or `confirmed`, a refused key and a failed halt are results with `isError: true` |
| An unknown tool is a protocol error (`-32602` in the specification's example) | `-32602` |
| A server with resources declares the `resources` capability | Declared, with `subscribe` and `listChanged` both `false` |
| A listed resource has `uri` and `name` | `uri`, `name`, `description` and `mimeType` |
| A resource that does not exist should be error `-32002` | `-32002` |

## The stdio transport

`sa-forge-signer-mcp` is the stdio side. It is not a server of its own: it relays each message to the service above, so the methods, the revisions and the gaps are the service's.

| Rule | The binary |
|---|---|
| Messages are JSON-RPC, one per line, with no newline inside a message | Each reply is written as one line, whatever the service's formatting |
| Standard output carries only protocol messages | Yes: errors and nothing else go to standard error |
| A notification gets no reply | None is written, also when the service cannot be reached |

When the service cannot be reached, or answers outside JSON-RPC (a wrong token is HTTP `401`), the binary writes a JSON-RPC error with code `-32000` for that request's `id`. A line that is not JSON gets `-32700` with a `null` id.

## Known gaps

- **`MCP-Protocol-Version` is not validated.** From `2025-06-18` a client sends this header on every request after `initialize`, and a server must answer an invalid or unsupported value with `400`. The service ignores the header, so it answers such a request as if the version were fine. A missing header is allowed: the specification has the server assume `2025-03-26`.
- **Batches are refused, also under `2025-03-26`.** That revision says an implementation must support receiving a JSON-RPC batch; `2025-06-18` removed batching. The service refuses an array body under every revision, so it does not fully conform to the oldest revision it negotiates.
- **`2026-07-28` is not implemented.**

Each is tracked in the [Plan](plan.md).

## The 2026-07-28 revision

The latest revision changes the protocol's shape, and the service does not speak it. From its changelog, the changes that touch what the service does:

- The `initialize` and `notifications/initialized` handshake is removed. Every request carries its protocol version and the client's capabilities in `_meta`, and a new `server/discover` method, which a server must implement, advertises versions, capabilities and identity.
- `ping` is removed, as are protocol-level sessions and the `GET` stream.
- Every POST must carry the `MCP-Protocol-Version` header and the `Mcp-Method` and `Mcp-Name` headers.
- Every result carries a required `resultType` field.
- A resource that does not exist is error `-32602`, and `-32002` must no longer be sent.

The service's handler is already stateless, which is the direction of this revision, but none of the above is built. How a client that speaks only `2026-07-28` behaves against the service has not been tested.

## Keeping this page true

A change to `src/serve/mcp.rs` or `src/mcp_cli.rs` that adds a method, a capability, a status or an error code updates this page in the same change. Before supporting a new revision, read its changelog page first: it lists what was removed as well as what was added.
