---
description: "MCP conformance: the specification the signer follows and where to read it, the one revision it speaks (2026-07-28), what the service's /mcp route and the stdio binary implement rule by rule, what an older client sees, and what is left out"
---

# MCP conformance

The signer speaks the [Model Context Protocol](https://modelcontextprotocol.io/specification/) with a handler written by hand (`src/serve/mcp.rs`), not an SDK ([Design decisions](design.md#the-service-is-synchronous-tiny_http-and-a-hand-written-mcp-handler) says why). This page is the record of what it implements, checked against the specification's pages on 2026-11-11. The Rust MCP SDK (`rmcp`) is a test-only dependency that checks the handler's messages ([Checked by the SDK](#checked-by-the-sdk)).

The specification to read is the one at `modelcontextprotocol.io`, which is built from the `modelcontextprotocol/specification` repository. Other sites reproduce it; when they differ, that one is right. Each revision has its own tree, such as `https://modelcontextprotocol.io/specification/2026-07-28/`, and its TypeScript schema is the source of truth for message shapes.

## Revision

The signer speaks **`2026-07-28`** and nothing earlier. That revision is stateless: there is no `initialize` handshake and no session, and every request carries its protocol version and the client's capabilities in `params._meta`.

A client of an earlier revision cannot use the signer. Its `initialize` is answered with error `-32022` (unsupported protocol version) naming `2026-07-28`, on HTTP with status `400`, so the client can say why. The specification gives such a client no way to move forward on its own. Claude Code speaks `2026-07-28` by default from version 2.1.274, by its own documentation.

## Scope

The service is an MCP server with two features: tools and resources. It offers no prompts, completions, subscriptions, tasks or extensions, never asks the client for input (every result is `complete`, never `input_required`), and sends no notifications. Its lists are short and fixed, so they are never paginated and never change.

| Method | Answer |
|---|---|
| `server/discover` | `supportedVersions` (`2026-07-28`), `capabilities` (`tools`, `resources`), and `instructions` |
| `tools/list` | `check`, `sign`, `key_show` and `halt`, each with an `inputSchema`, always in that order |
| `tools/call` | One text content block and `isError` ([Serving over HTTP and MCP](../serve.md)) |
| `resources/list`, `resources/read` | Four static texts ([Resources and the skill](../serve.md#resources-and-the-skill)) |
| any notification | Accepted and ignored |
| `initialize` | Error `-32022`, as above |
| anything else, `ping` included | Error `-32601`, method not found, on HTTP with status `404` |

Every result carries `resultType: "complete"` and names the server in `_meta["io.modelcontextprotocol/serverInfo"]`. The discovery result, both lists and a resource read also carry the caching hints the revision requires: `ttlMs` of 300000 and `cacheScope` `private`, since every answer is behind a bearer token.

## A request

| Rule | The service |
|---|---|
| A request's `id` is a string or an integer, never `null` | Anything else is error `-32600`, status `400` |
| `params._meta` carries `io.modelcontextprotocol/protocolVersion` and `io.modelcontextprotocol/clientCapabilities`; a request without them is error `-32602`, status `400` | Yes. The capabilities must be an object; the service needs none, so it reads no further |
| A version the server does not implement is error `-32022` with `data.supported` and `data.requested`, status `400` | Yes |
| A server must not rely on earlier requests for context | The handler keeps no state between requests |

## The HTTP transport

`POST /mcp` on `sa-forge-signer-serve` follows the Streamable HTTP transport, in its plain form: one JSON object per reply, no event stream.

| Rule | The service |
|---|---|
| A request is answered with `Content-Type: application/json` or an event stream | Always `application/json` |
| An accepted notification gets `202 Accepted` with no body | Yes |
| Every POST carries `MCP-Protocol-Version`, matching the body's `_meta`; a missing or different one is error `-32020` (header mismatch), status `400` | Yes |
| Every request carries `Mcp-Method`, matching the body's `method` | Yes: missing or different is `-32020`, status `400` |
| `tools/call` and `resources/read` carry `Mcp-Name`, matching `params.name` or `params.uri` | Yes: missing or different is `-32020`, status `400`. A value sent as `=?base64?...?=` is decoded before it is compared |
| A method the server does not implement is `404 Not Found` with error `-32601` | Yes |
| A server of this revision alone answers `GET` and `DELETE` with `405 Method Not Allowed` | Yes |
| The `Origin` header must be validated, and an invalid one answered with `403 Forbidden` | Yes: `Origin` and `Host` must name an entry in `allowed_hosts` |
| `Mcp-Session-Id` and `Last-Event-ID` are ignored | They are never read |

The checks run in this order: the body is JSON and one message, `initialize`, the `_meta` fields, the headers against the body, then the version. Authentication is outside the specification's own scheme: the service takes a bearer token from its config, not OAuth. A request without a known token is `401` before the body is read, with a plain JSON body, not a JSON-RPC error.

Tool arguments are not mirrored into `Mcp-Param-*` headers: no tool marks a parameter with `x-mcp-header`.

## Tools and resources

| Rule | The service |
|---|---|
| A tool that ran and failed reports it in its result with `isError: true`; a protocol problem is a JSON-RPC error | A report whose `outcome` is not `ok` or `confirmed`, a refused key and a failed halt are results with `isError: true` |
| An unknown tool is error `-32602` | Yes |
| A server with tools or resources declares the capability | Both declared, as empty objects: no `listChanged`, no `subscribe` |
| A resource that does not exist is error `-32602`; `-32002` must not be sent | `-32602` with the `uri` in `data` |
| A listed resource has `uri` and `name` | `uri`, `name`, `description` and `mimeType` |

## The stdio transport

`sa-forge-signer-mcp` is the stdio side. It is not a server of its own: it relays each message to the service above, so the methods and the rules are the service's.

| Rule | The binary |
|---|---|
| Messages are JSON-RPC, one per line, with no newline inside a message | Each reply is written as one line, whatever the service's formatting |
| Standard output carries only protocol messages | Yes: anything else goes to standard error |
| A notification gets no reply | None is written, also when the service cannot be reached |
| On stdio the request metadata is only in `_meta`; there are no headers | The binary builds the three HTTP headers from each message's body before forwarding it |
| A client probing with `server/discover` gets a discovery result or a recognised error | The service's answer is passed through, as is its `-32022` for an `initialize` |

When the service cannot be reached, or answers outside JSON-RPC (a wrong token is HTTP `401`), the binary writes a JSON-RPC error with code `-32603` for that request's `id`. A line that is not JSON gets `-32700` with no `id`.

## Checked by the SDK

`src/serve/mcp_sdk_tests.rs` reads the handler's messages back with the types of `rmcp`, the Rust MCP SDK, pinned to one version as a dev-dependency. The binaries do not link it. The tests check that:

- the requests the tests send parse as the SDK's client requests, so they model a real client;
- the results of `server/discover`, `tools/list`, `tools/call`, `resources/list` and `resources/read` parse as the SDK's types for them, with `resultType`, the caching hints and the server's name where the SDK reads them;
- each error parses as the SDK's error with the code the SDK defines for it, and the unsupported-version error equals the one the SDK itself builds.

## Not checked

- The SDK's types are looser than the specification in places: fields the `2026-07-28` schema requires, such as `resultType` on a list, are optional in the SDK because it also models earlier revisions. The tests assert those fields themselves.
- No MCP client has been run against a live service. The SDK's client is not used: it would bring an HTTP stack into the tests for little beyond what the type checks give. The tests in `src/serve/` and `src/mcp_cli.rs` send requests built by hand.
- The specification's TypeScript schema was read only in part; the pages for the base protocol, versioning, discovery, both transports, tools, resources, pagination and the changelog were read in full.

## Keeping this page true

A change to `src/serve/mcp.rs`, `src/mcp_wire.rs` or `src/mcp_cli.rs` that adds a method, a header, a status or an error code updates this page and `src/serve/mcp_sdk_tests.rs` in the same change. When `rmcp` is bumped, a test that stops passing is a difference between the handler and the SDK's reading of the specification: find out which is right before changing either. Before supporting a new revision, read its changelog page first: it lists what was removed as well as what was added.
