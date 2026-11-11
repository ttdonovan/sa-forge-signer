//! The handler's messages, read back with the Rust MCP SDK's own types (`rmcp`, a dev-dependency
//! only). The handler is written by hand; these tests are what would notice it drifting from
//! revision 2026-07-28 as the SDK models it.

use rmcp::model::{
    CacheScope, CallToolResult, ClientJsonRpcMessage, DiscoverResult, ErrorCode, ErrorData,
    JsonRpcError, JsonRpcMessage, JsonRpcResponse, ListResourcesResult, ListToolsResult,
    ProtocolVersion, ReadResourceResult, ResultType,
};
use serde_json::{Value, json};

use super::mcp::{Headers, handle};
use crate::config::{Config, TokenConfig};
use crate::mcp_wire::{META_CLIENT_CAPABILITIES, META_VERSION, VERSION, name_field};

const KEY: &str = "7JJhMrnvgqxS6MW59QQcr2Ki1NjZ7SrAjST4YtAFE8zR";

fn config() -> (Config, TokenConfig) {
    let cfg = Config::load_str(&format!(
        "[keys.mine]\nclass = \"session\"\npubkey = \"{KEY}\"\n"
    ))
    .unwrap();
    let token = TokenConfig {
        name: "t".to_owned(),
        sha256: String::new(),
        keys: vec!["mine".to_owned()],
    };
    (cfg, token)
}

/// A request as a client of this revision sends it, and the headers that mirror it.
fn request(method: &str, mut params: Value) -> (Value, Headers) {
    params["_meta"] = json!({META_VERSION: VERSION, META_CLIENT_CAPABILITIES: {}});
    let name = name_field(method).and_then(|f| params[f].as_str().map(str::to_owned));
    let headers = Headers {
        version: Some(VERSION.to_owned()),
        method: Some(method.to_owned()),
        name,
    };
    (
        json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}),
        headers,
    )
}

/// The handler's reply to `method`: its HTTP status and body.
fn call(method: &str, params: Value) -> (u16, Value) {
    let (cfg, token) = config();
    let (body, headers) = request(method, params);
    let reply = handle(&cfg, &token, &body.to_string(), &headers);
    (reply.status, reply.body.unwrap())
}

fn result<T: serde::de::DeserializeOwned>(method: &str, params: Value) -> T {
    let (status, body) = call(method, params);
    assert_eq!(status, 200, "{body}");
    serde_json::from_value::<JsonRpcResponse<T>>(body.clone())
        .unwrap_or_else(|e| panic!("{method}: {e}: {body}"))
        .result
}

fn error(body: &Value) -> ErrorData {
    serde_json::from_value::<JsonRpcError>(body.clone())
        .unwrap_or_else(|e| panic!("{e}: {body}"))
        .error
}

#[test]
fn the_requests_these_tests_send_are_ones_the_sdk_accepts() {
    for (method, params) in [
        ("server/discover", json!({})),
        ("tools/list", json!({})),
        (
            "tools/call",
            json!({"name": "key_show", "arguments": {"name": "mine"}}),
        ),
        ("resources/list", json!({})),
        (
            "resources/read",
            json!({"uri": "sa-forge-signer://llms.txt"}),
        ),
    ] {
        let (body, _) = request(method, params);
        let parsed: ClientJsonRpcMessage = serde_json::from_value(body.clone())
            .unwrap_or_else(|e| panic!("{method}: {e}: {body}"));
        assert!(matches!(parsed, JsonRpcMessage::Request(_)), "{method}");
    }
}

#[test]
fn the_sdk_reads_the_discovery_result() {
    let d: DiscoverResult = result("server/discover", json!({}));
    assert_eq!(d.result_type, ResultType::COMPLETE);
    assert_eq!(d.supported_versions, [ProtocolVersion::V_2026_07_28]);
    assert_eq!(ProtocolVersion::LATEST.to_string(), VERSION);
    assert!(d.capabilities.tools.is_some() && d.capabilities.resources.is_some());
    assert!(d.capabilities.prompts.is_none());
    assert!(d.instructions.is_some());
    assert_eq!(d.cache_scope, CacheScope::Private);
    assert!(d.ttl_ms > 0);
    let server = d.server_info().unwrap();
    assert_eq!(server.name, "sa-forge-signer");
    assert_eq!(server.version, env!("CARGO_PKG_VERSION"));
}

#[test]
fn the_sdk_reads_the_lists_and_a_resource() {
    let tools: ListToolsResult = result("tools/list", json!({}));
    let names: Vec<&str> = tools.tools.iter().map(|t| t.name.as_ref()).collect();
    assert_eq!(names, ["check", "sign", "key_show", "halt"]);
    assert_eq!(tools.result_type, Some(ResultType::COMPLETE));
    assert_eq!(tools.cache_scope, Some(CacheScope::Private));
    assert!(tools.ttl_ms.is_some() && tools.next_cursor.is_none());

    let listed: ListResourcesResult = result("resources/list", json!({}));
    assert_eq!(listed.resources.len(), 4);
    assert_eq!(listed.result_type, Some(ResultType::COMPLETE));
    assert!(listed.ttl_ms.is_some());

    let read: ReadResourceResult = result(
        "resources/read",
        json!({"uri": "sa-forge-signer://llms.txt"}),
    );
    assert_eq!(read.contents.len(), 1);
    assert_eq!(read.result_type, Some(ResultType::COMPLETE));
    assert!(read.ttl_ms.is_some());
}

#[test]
fn the_sdk_reads_a_tool_result() {
    // A key this token does not list: the tool ran and refused, which is a result, not an error.
    let refused: CallToolResult = result(
        "tools/call",
        json!({"name": "key_show", "arguments": {"name": "other"}}),
    );
    assert_eq!(refused.is_error, Some(true));
    assert_eq!(refused.result_type, Some(ResultType::COMPLETE));
    assert_eq!(refused.content.len(), 1);
}

#[test]
fn the_sdk_reads_each_error_with_the_code_it_defines() {
    let (status, body) = call("tools/call", json!({"name": "resume", "arguments": {}}));
    assert_eq!(
        (status, error(&body).code),
        (200, ErrorCode::INVALID_PARAMS)
    );

    let (status, body) = call("resources/read", json!({"uri": "sa-forge-signer://nope"}));
    let missing = error(&body);
    assert_eq!((status, missing.code), (200, ErrorCode::INVALID_PARAMS));
    assert_ne!(missing.code, ErrorCode::RESOURCE_NOT_FOUND);

    let (status, body) = call("prompts/list", json!({}));
    assert_eq!(
        (status, error(&body).code),
        (404, ErrorCode::METHOD_NOT_FOUND)
    );

    let (cfg, token) = config();
    let ask = |body: &Value, headers: &Headers| {
        let reply = handle(&cfg, &token, &body.to_string(), headers);
        (reply.status, error(&reply.body.unwrap()))
    };
    let (body, mut headers) = request("tools/list", json!({}));
    headers.method = Some("tools/call".to_owned());
    let (status, mismatch) = ask(&body, &headers);
    assert_eq!((status, mismatch.code), (400, ErrorCode::HEADER_MISMATCH));

    // The same error the SDK builds for a version it does not speak.
    let mut old = body;
    old["params"]["_meta"][META_VERSION] = json!("2025-11-25");
    let headers = Headers {
        version: Some("2025-11-25".to_owned()),
        method: Some("tools/list".to_owned()),
        name: None,
    };
    let (status, unsupported) = ask(&old, &headers);
    assert_eq!(status, 400);
    assert_eq!(
        unsupported,
        ErrorData::unsupported_protocol_version(
            ProtocolVersion::V_2025_11_25,
            &[ProtocolVersion::V_2026_07_28]
        )
    );

    // An earlier revision's handshake, and a body that is not JSON (which has no id to echo).
    let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
        "params": {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "old", "version": "0"}}});
    let (status, refused) = ask(&init, &Headers::default());
    assert_eq!(
        (status, refused.code),
        (400, ErrorCode::UNSUPPORTED_PROTOCOL_VERSION)
    );
    let garbled = handle(&cfg, &token, "not json", &Headers::default());
    let parsed: JsonRpcError = serde_json::from_value(garbled.body.unwrap()).unwrap();
    assert_eq!(
        (garbled.status, parsed.error.code),
        (400, ErrorCode::PARSE_ERROR)
    );
    assert!(parsed.id.is_none());
}
