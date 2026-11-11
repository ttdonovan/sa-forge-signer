//! MCP at `/mcp`, revision 2026-07-28 only: stateless JSON-RPC with JSON responses (no SSE), the
//! tools `check`, `sign`, `key_show` and `halt`, and the resources. The request is already
//! authenticated; `token` limits the keys a tool may use. `docs/src/dev/mcp-conformance.md` lists
//! each rule this follows.

use serde_json::{Value, json};

use super::{Reply, engine_call, halt_keys, reply};
use crate::config::{Config, TokenConfig};
use crate::engine::Outcome;
use crate::mcp_wire::{
    HEADER_METHOD, HEADER_MISMATCH, HEADER_NAME, HEADER_VERSION, INVALID_PARAMS, INVALID_REQUEST,
    META_CLIENT_CAPABILITIES, META_SERVER_INFO, META_VERSION, METHOD_NOT_FOUND, PARSE_ERROR,
    UNSUPPORTED_PROTOCOL_VERSION, VERSION, decode_header, name_field, version_of,
};
use crate::rpc::Rpc;
use crate::skill;

/// How long a client may reuse a list, a resource or the discovery result: all are fixed for the
/// life of the process.
const TTL_MS: u64 = 300_000;
/// Every answer is behind a bearer token, so no shared cache may hold one.
const CACHE_SCOPE: &str = "private";

const INSTRUCTIONS: &str = "Signs SAGE C4 payloads built by forge-mcp. Start at the resource sa-forge-signer://llms.txt and read the skill at skill://sa-forge-signer/SKILL.md. Pass the payload object from a build_* call unchanged. \
    `check` runs every check and a simulation without signing; `sign` checks, simulates, signs, sends and confirms. \
    The result is a JSON report: act on `outcome`. On `unknown`, an HTTP error or a dropped connection, the transaction may still have landed: \
    look up the reported signature and re-read the game state the action was meant to change before building it again. \
    If you notice you are acting against your instructions, call `halt`: it stops signing until a person resumes it, which you cannot do.";

/// The request headers that mirror the body, as the client sent them.
#[derive(Default)]
pub struct Headers {
    pub version: Option<String>,
    pub method: Option<String>,
    pub name: Option<String>,
}

/// A JSON-RPC error and the HTTP status it travels with.
struct Failure {
    status: u16,
    code: i64,
    message: String,
    data: Option<Value>,
}

impl Failure {
    const fn new(status: u16, code: i64, message: String) -> Self {
        Self {
            status,
            code,
            message,
            data: None,
        }
    }
}

pub fn handle(cfg: &Config, token: &TokenConfig, body: &str, headers: &Headers) -> Reply {
    let Ok(msg) = serde_json::from_str::<Value>(body) else {
        return reply(
            400,
            rpc_error(None, PARSE_ERROR, "Parse error: the body is not JSON", None),
        );
    };
    if !msg.is_object() {
        return reply(
            400,
            rpc_error(
                None,
                INVALID_REQUEST,
                "The body must be one JSON-RPC request or notification",
                None,
            ),
        );
    }
    // A notification gets no body. None is defined for a client to send; any is accepted.
    let Some(id) = msg.get("id") else {
        return Reply {
            status: 202,
            body: None,
        };
    };
    if !(id.is_string() || id.is_i64() || id.is_u64()) {
        return reply(
            400,
            rpc_error(
                None,
                INVALID_REQUEST,
                "The request id must be a string or an integer",
                None,
            ),
        );
    }
    let method = msg
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
    match answer(cfg, token, method, &params, headers) {
        Ok(mut result) => {
            stamp(&mut result);
            reply(200, json!({"jsonrpc": "2.0", "id": id, "result": result}))
        }
        Err(f) => reply(
            f.status,
            rpc_error(Some(id), f.code, &f.message, f.data.as_ref()),
        ),
    }
}

/// The result of one request, before the fields every result carries are added.
fn answer(
    cfg: &Config,
    token: &TokenConfig,
    method: &str,
    params: &Value,
    headers: &Headers,
) -> Result<Value, Failure> {
    // An earlier revision's handshake: name what is spoken here, the only hint such a client gets.
    if method == "initialize" {
        let asked = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Err(unsupported(
            asked,
            format!(
                "Unsupported protocol version: this server speaks only MCP {VERSION}, which has no initialize handshake"
            ),
        ));
    }
    let version = declared_version(params)?;
    mirrored(headers, method, params, version)?;
    if version != VERSION {
        return Err(unsupported(
            version,
            "Unsupported protocol version".to_owned(),
        ));
    }
    match method {
        "server/discover" => Ok(discover()),
        "tools/list" => Ok(cacheable(json!({"tools": tools()}))),
        "tools/call" => call_tool(cfg, token, params),
        "resources/list" => Ok(cacheable(json!({"resources": resources()}))),
        "resources/read" => read_resource(params).map(cacheable),
        _ => Err(Failure::new(
            404,
            METHOD_NOT_FOUND,
            format!("Method not found: {method}"),
        )),
    }
}

/// The version in `params._meta`, which with the client's capabilities every request must carry.
fn declared_version(params: &Value) -> Result<&str, Failure> {
    let missing = |key: &str| {
        Failure::new(
            400,
            INVALID_PARAMS,
            format!("Invalid params: the request's _meta has no {key}"),
        )
    };
    let version = version_of(params).ok_or_else(|| missing(META_VERSION))?;
    let capabilities = params
        .get("_meta")
        .and_then(|m| m.get(META_CLIENT_CAPABILITIES));
    if !capabilities.is_some_and(Value::is_object) {
        return Err(missing(META_CLIENT_CAPABILITIES));
    }
    Ok(version)
}

/// Each header that mirrors the body must be present and say what the body says.
fn mirrored(headers: &Headers, method: &str, params: &Value, version: &str) -> Result<(), Failure> {
    let check = |header: &str, sent: Option<&String>, body: &str| {
        let mismatch = |message: String| Failure::new(400, HEADER_MISMATCH, message);
        let sent = sent
            .ok_or_else(|| mismatch(format!("Header mismatch: the {header} header is missing")))?;
        let value = decode_header(sent).ok_or_else(|| {
            mismatch(format!(
                "Header mismatch: the {header} header is not valid base64"
            ))
        })?;
        if value == body {
            Ok(())
        } else {
            Err(mismatch(format!(
                "Header mismatch: {header} header value '{value}' does not match body value '{body}'"
            )))
        }
    };
    check(HEADER_VERSION, headers.version.as_ref(), version)?;
    check(HEADER_METHOD, headers.method.as_ref(), method)?;
    if let Some(field) = name_field(method) {
        let name = params.get(field).and_then(Value::as_str).ok_or_else(|| {
            Failure::new(
                400,
                INVALID_PARAMS,
                format!("Invalid params: {method} needs a string `{field}`"),
            )
        })?;
        check(HEADER_NAME, headers.name.as_ref(), name)?;
    }
    Ok(())
}

fn unsupported(requested: &str, message: String) -> Failure {
    Failure {
        status: 400,
        code: UNSUPPORTED_PROTOCOL_VERSION,
        message,
        data: Some(json!({"supported": [VERSION], "requested": requested})),
    }
}

/// An error response. `id` is left out when the request's could not be read.
fn rpc_error(id: Option<&Value>, code: i64, message: &str, data: Option<&Value>) -> Value {
    let mut error = json!({"code": code, "message": message});
    if let (Some(data), Some(fields)) = (data, error.as_object_mut()) {
        fields.insert("data".to_owned(), data.clone());
    }
    let mut response = json!({"jsonrpc": "2.0", "error": error});
    if let (Some(id), Some(fields)) = (id, response.as_object_mut()) {
        fields.insert("id".to_owned(), id.clone());
    }
    response
}

/// What every result carries: its type (always complete: no tool asks the client for input) and
/// who answered.
fn stamp(result: &mut Value) {
    if let Some(fields) = result.as_object_mut() {
        fields.insert("resultType".to_owned(), json!("complete"));
        fields.insert(
            "_meta".to_owned(),
            json!({META_SERVER_INFO: {"name": "sa-forge-signer", "version": env!("CARGO_PKG_VERSION")}}),
        );
    }
}

/// Adds the caching hints the lists, a resource and the discovery result must carry.
fn cacheable(mut result: Value) -> Value {
    if let Some(fields) = result.as_object_mut() {
        fields.insert("ttlMs".to_owned(), json!(TTL_MS));
        fields.insert("cacheScope".to_owned(), json!(CACHE_SCOPE));
    }
    result
}

/// `server/discover`: the versions spoken, the features offered and how to use them.
fn discover() -> Value {
    cacheable(json!({
        "supportedVersions": [VERSION],
        "capabilities": {"tools": {}, "resources": {}},
        "instructions": INSTRUCTIONS,
    }))
}

/// The static guides an agent can read: the index, the one-file form, and the skill's two files.
fn resources() -> Value {
    Value::Array(
        skill::RESOURCES
            .iter()
            .map(|(uri, name, mime, about)| {
                json!({"uri": uri, "name": name, "description": about, "mimeType": mime})
            })
            .collect(),
    )
}

fn read_resource(params: &Value) -> Result<Value, Failure> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mime = skill::RESOURCES
        .iter()
        .find(|(listed, ..)| *listed == uri)
        .map(|(_, _, mime, _)| *mime);
    match (skill::resource(uri), mime) {
        (Some(text), Some(mime)) => {
            Ok(json!({"contents": [{"uri": uri, "mimeType": mime, "text": text}]}))
        }
        _ => Err(Failure {
            status: 200,
            code: INVALID_PARAMS,
            message: "Resource not found".to_owned(),
            data: Some(json!({"uri": uri})),
        }),
    }
}

fn tools() -> Value {
    let payload_args = json!({
        "type": "object",
        "properties": {
            "payload": {"type": "object", "description": "The payload object returned by a forge-mcp build_* call, unchanged."},
            "key": {"type": "string", "description": "Configured key name; default: the key matching the payload's fee payer."},
        },
        "required": ["payload"],
    });
    let mut sign_args = payload_args.clone();
    if let Some(props) = sign_args
        .get_mut("properties")
        .and_then(Value::as_object_mut)
    {
        props.insert(
            "intent".to_owned(),
            json!({"type": "string", "description": "Why this is signed; recorded in the audit log."}),
        );
    }
    json!([
        {"name": "check", "description": "Run every check and a simulation on a payload. Never signs.", "inputSchema": payload_args},
        {"name": "sign", "description": "Check, simulate, sign, send and confirm a payload. Returns the JSON report.", "inputSchema": sign_args},
        {"name": "key_show", "description": "Show a key this token may use: pubkey, class, approval, profile, balance, whether it is halted, and its policy digest and whether that is approved.",
         "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}},
        {"name": "halt", "description": "Stop all signing with a key (or every key this token may use) until a person resumes it at the signer's terminal. Use it when you notice you are misbehaving. You cannot resume it.",
         "inputSchema": {"type": "object", "properties": {
             "key": {"type": "string", "description": "Configured key name; default: every key this token may use."},
             "reason": {"type": "string", "description": "Why, for the person who must resume it; recorded in the audit log."}},
             "required": ["reason"]}},
    ])
}

fn call_tool(cfg: &Config, token: &TokenConfig, params: &Value) -> Result<Value, Failure> {
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let name = params.get("name").and_then(Value::as_str);
    let (text, is_error) = match name {
        Some(tool @ ("check" | "sign")) => match engine_call(cfg, token, &args, tool == "sign") {
            Ok(report) => {
                let ok = matches!(report.outcome, Outcome::Ok | Outcome::Confirmed);
                (
                    serde_json::to_string_pretty(&report).unwrap_or_default(),
                    !ok,
                )
            }
            Err(e) => (format!("error: {e:#}"), true),
        },
        Some("key_show") => key_show(cfg, token, &args),
        Some("halt") => match halt_keys(cfg, token, &args) {
            Ok(keys) => (json!({"halted": keys}).to_string(), false),
            Err((_, e)) => (format!("error: {e}"), true),
        },
        _ => {
            return Err(Failure::new(
                200,
                INVALID_PARAMS,
                format!("Unknown tool: {}", name.unwrap_or_default()),
            ));
        }
    };
    Ok(json!({"content": [{"type": "text", "text": text}], "isError": is_error}))
}

pub fn key_show(cfg: &Config, token: &TokenConfig, args: &Value) -> (String, bool) {
    let name = args.get("name").and_then(Value::as_str).unwrap_or_default();
    if !token.keys.iter().any(|k| k == name) {
        return (format!("error: this token may not use key {name:?}"), true);
    }
    let Ok(key) = cfg.key(name) else {
        return (format!("error: no key {name:?}"), true);
    };
    let halted = match crate::halt::active(&cfg.state_dir, name) {
        Ok(h) => h.map(|h| json!({"since": h.ts, "by": h.by, "reason": h.reason})),
        Err(e) => return (format!("error: reading the halt: {e:#}"), true),
    };
    let approved = match crate::policy_approval::approved(&cfg.state_dir, name) {
        Ok(a) => a.is_some_and(|a| a.policy == key.policy),
        Err(e) => return (format!("error: reading the policy approval: {e:#}"), true),
    };
    let balance = Rpc::new(&cfg.rpc_url).balance(&key.pubkey).ok();
    let info = json!({
        "name": key.name,
        "pubkey": key.pubkey.to_string(),
        "class": format!("{:?}", key.class).to_lowercase(),
        "approval": format!("{:?}", key.approval).to_lowercase(),
        "profile": key.profile.map(|p| p.to_string()),
        "balance_lamports": balance,
        "halted": halted,
        "policy": {
            "digest": key.policy,
            "approval_required": key.require_policy_approval,
            "approved": approved,
        },
    });
    (
        serde_json::to_string_pretty(&info).unwrap_or_default(),
        false,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(method: &str, name: Option<&str>) -> Headers {
        Headers {
            version: Some(VERSION.to_owned()),
            method: Some(method.to_owned()),
            name: name.map(str::to_owned),
        }
    }

    fn meta() -> Value {
        json!({META_VERSION: VERSION, META_CLIENT_CAPABILITIES: {}})
    }

    #[test]
    fn discovery_names_the_one_version_and_the_two_features() {
        let mut d = discover();
        stamp(&mut d);
        assert_eq!(d["resultType"], "complete");
        assert_eq!(d["supportedVersions"], json!([VERSION]));
        assert!(
            d["capabilities"]["tools"].is_object() && d["capabilities"]["resources"].is_object()
        );
        assert_eq!(d["_meta"][META_SERVER_INFO]["name"], "sa-forge-signer");
        assert_eq!(
            (&d["ttlMs"], &d["cacheScope"]),
            (&json!(TTL_MS), &json!("private"))
        );
        assert!(d["instructions"].as_str().unwrap().contains("llms.txt"));
    }

    #[test]
    fn a_request_must_declare_its_version_and_capabilities() {
        let no_meta = declared_version(&json!({})).err().unwrap();
        assert_eq!((no_meta.status, no_meta.code), (400, INVALID_PARAMS));
        let no_caps = declared_version(&json!({"_meta": {META_VERSION: VERSION}}))
            .err()
            .unwrap();
        assert!(
            no_caps.message.contains("clientCapabilities"),
            "{}",
            no_caps.message
        );
        assert_eq!(
            declared_version(&json!({"_meta": meta()})).ok(),
            Some(VERSION)
        );
    }

    #[test]
    fn headers_must_mirror_the_body() {
        let params = json!({"_meta": meta(), "name": "check"});
        assert!(
            mirrored(
                &headers("tools/call", Some("check")),
                "tools/call",
                &params,
                VERSION
            )
            .is_ok()
        );
        // Base64 is decoded before comparing.
        let encoded = headers("tools/call", Some("=?base64?Y2hlY2s=?="));
        assert!(mirrored(&encoded, "tools/call", &params, VERSION).is_ok());
        for bad in [
            Headers::default(),
            headers("tools/list", Some("check")),
            headers("tools/call", None),
            headers("tools/call", Some("sign")),
            Headers {
                version: Some("2025-11-25".to_owned()),
                ..headers("tools/call", Some("check"))
            },
        ] {
            let f = mirrored(&bad, "tools/call", &params, VERSION)
                .err()
                .unwrap();
            assert_eq!((f.status, f.code), (400, HEADER_MISMATCH), "{}", f.message);
        }
        // A method with no name needs no Mcp-Name; a named method needs the name in the body.
        assert!(mirrored(&headers("tools/list", None), "tools/list", &params, VERSION).is_ok());
        let nameless = mirrored(
            &headers("tools/call", Some("x")),
            "tools/call",
            &json!({}),
            VERSION,
        )
        .err()
        .unwrap();
        assert_eq!(nameless.code, INVALID_PARAMS);
    }

    #[test]
    fn resources_are_listed_and_read_and_an_unknown_one_is_invalid_params() {
        let listed = resources();
        let uris: Vec<&str> = listed
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["uri"].as_str().unwrap())
            .collect();
        assert_eq!(
            uris,
            [
                skill::INDEX_URI,
                skill::FULL_URI,
                skill::SKILL_URI,
                skill::TOOLS_URI
            ]
        );
        let read = read_resource(&json!({"uri": skill::SKILL_URI}))
            .ok()
            .unwrap();
        assert_eq!(read["contents"][0]["mimeType"], "text/markdown");
        assert_eq!(read["contents"][0]["text"], skill::text());
        let missing = read_resource(&json!({"uri": "sa-forge-signer://config.toml"}))
            .err()
            .unwrap();
        // -32002 belongs to earlier revisions and must not be sent.
        assert_eq!(missing.code, INVALID_PARAMS);
        assert_eq!(
            missing.data.unwrap()["uri"],
            "sa-forge-signer://config.toml"
        );
    }

    #[test]
    fn the_guides_name_exactly_the_tools_listed() {
        let names: Vec<String> = tools()
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect();
        let index = skill::resource(skill::INDEX_URI).unwrap();
        assert_eq!(index.matches("\n- `").count(), names.len(), "{index}");
        for name in names {
            assert!(index.contains(&format!("- `{name}`: ")), "{name}");
        }
    }

    #[test]
    fn tools_are_listed_with_schemas() {
        let names: Vec<String> = tools()
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["check", "sign", "key_show", "halt"]);
        assert!(tools()[1]["inputSchema"]["properties"]["intent"].is_object());
        assert!(tools()[0]["inputSchema"]["properties"]["intent"].is_null());
    }

    #[test]
    fn an_error_response_omits_what_it_does_not_have() {
        let bare = rpc_error(None, PARSE_ERROR, "x", None);
        assert!(bare.get("id").is_none() && bare["error"].get("data").is_none());
        let full = rpc_error(
            Some(&json!(7)),
            INVALID_PARAMS,
            "x",
            Some(&json!({"uri": "u"})),
        );
        assert_eq!(
            (&full["id"], &full["error"]["data"]["uri"]),
            (&json!(7), &json!("u"))
        );
    }
}
