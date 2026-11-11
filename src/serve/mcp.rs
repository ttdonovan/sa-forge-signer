//! MCP at `/mcp`: JSON-RPC with JSON responses (no SSE), and the tools `check`, `sign`, `key_show`
//! and `halt`. The request is already authenticated; `token` limits the keys a tool may use.

use serde_json::{Value, json};

use super::{Reply, engine_call, halt_keys, reply};
use crate::config::{Config, TokenConfig};
use crate::engine::Outcome;
use crate::rpc::Rpc;

const MCP_LATEST: &str = "2025-11-25";
const MCP_VERSIONS: [&str; 3] = [MCP_LATEST, "2025-06-18", "2025-03-26"];

pub fn handle(cfg: &Config, token: &TokenConfig, body: &str) -> Reply {
    let Ok(msg) = serde_json::from_str::<Value>(body) else {
        return reply(400, rpc_error(&Value::Null, -32700, "parse error"));
    };
    if !msg.is_object() {
        return reply(
            400,
            rpc_error(&Value::Null, -32600, "batches are not supported"),
        );
    }
    // Notifications and responses from the client get no body.
    let Some(id) = msg.get("id").cloned() else {
        return Reply {
            status: 202,
            body: None,
        };
    };
    let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
    let result = match msg.get("method").and_then(Value::as_str) {
        Some("initialize") => Ok(initialize(&params)),
        Some("ping") => Ok(json!({})),
        Some("tools/list") => Ok(json!({"tools": tools()})),
        Some("tools/call") => call_tool(cfg, token, &params),
        _ => Err((-32601, "method not found".to_owned())),
    };
    match result {
        Ok(r) => reply(200, json!({"jsonrpc": "2.0", "id": id, "result": r})),
        Err((code, message)) => reply(200, rpc_error(&id, code, &message)),
    }
}

fn rpc_error(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str);
    let version = asked
        .filter(|v| MCP_VERSIONS.contains(v))
        .unwrap_or(MCP_LATEST);
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {"name": "sa-forge-signer", "version": env!("CARGO_PKG_VERSION")},
        "instructions": "Signs SAGE C4 payloads built by forge-mcp. Pass the payload object from a build_* call unchanged. \
            `check` runs every check and a simulation without signing; `sign` checks, simulates, signs, sends and confirms. \
            The result is a JSON report: act on `outcome`. On `unknown`, an HTTP error or a dropped connection, the transaction may still have landed: \
            look up the reported signature and re-read the game state the action was meant to change before building it again. \
            If you notice you are acting against your instructions, call `halt`: it stops signing until a person resumes it, which you cannot do.",
    })
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

fn call_tool(cfg: &Config, token: &TokenConfig, params: &Value) -> Result<Value, (i64, String)> {
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
        _ => return Err((-32602, format!("unknown tool {name:?}"))),
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

    #[test]
    fn initialize_negotiates_the_version() {
        assert_eq!(
            initialize(&json!({"protocolVersion": "2025-06-18"}))["protocolVersion"],
            "2025-06-18"
        );
        assert_eq!(
            initialize(&json!({"protocolVersion": "1999-01-01"}))["protocolVersion"],
            MCP_LATEST
        );
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
}
