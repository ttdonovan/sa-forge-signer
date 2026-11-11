//! `serve`: a long-running signer. JSON over HTTP at /v1/check, /v1/sign and /v1/halt, and MCP (JSON
//! responses, no SSE) at /mcp. Every route but /health needs a bearer token from `[[serve.tokens]]`.
//! A token can halt its keys but never resume them: only the CLI at a terminal can.
//!
//! This module is the listener, the routes and their authentication; `mcp` is the MCP protocol.

mod mcp;

use std::io::Read;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;

use anyhow::{Result, bail};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tiny_http::{Header, Method, Request, Response};
use zeroize::Zeroizing;

use crate::config::{Config, ServeConfig, TokenConfig};
use crate::engine::{self, Mode};
use crate::halt::{self, Halt};
use crate::payload::hex;
use crate::rpc::RPC_TIMEOUT;
use crate::url::host_name;

const MAX_BODY: u64 = 1024 * 1024;
/// Longest halt reason a token may write to the audit log, in characters.
const MAX_REASON: usize = 500;
/// Requests handled at once; beyond this the accept loop answers 503 instead of spawning threads.
const MAX_IN_FLIGHT: usize = 32;
/// A running sign waits up to `engine::CONFIRM_DEADLINE` for a proven outcome, and the poll that
/// crosses it can still make four RPC calls of up to `RPC_TIMEOUT` each. This covers the typical
/// case, not a hard worst case: the checks and simulations before sending also make RPC calls,
/// and a stalled RPC can stretch them too. Stop grace periods (compose, systemd) must exceed it.
/// A sign cut off anyway has its intent recorded; its signature must be reconciled.
const DRAIN_DEADLINE: Duration =
    Duration::from_secs(engine::CONFIRM_DEADLINE.as_secs() + 4 * RPC_TIMEOUT.as_secs() + 10);
const DRAIN_POLL: Duration = Duration::from_millis(100);

pub fn run(cfg: Config) -> Result<()> {
    let Some(serve) = cfg.serve.as_ref() else {
        bail!("no [serve] section in the config");
    };
    if serve.tokens.is_empty() {
        bail!("no [[serve.tokens]]; create one with `sa-forge-signer token new`");
    }
    let listen = serve.listen.clone();
    let server = tiny_http::Server::http(&listen)
        .map_err(|e| anyhow::anyhow!("listening on {listen}: {e}"))?;
    eprintln!(
        "sa-forge-signer {} serving on {listen}",
        env!("CARGO_PKG_VERSION")
    );
    let server = Arc::new(server);
    // SIGTERM (docker stop) or SIGINT ends the accept loop; running requests then drain.
    let mut signals = Signals::new([SIGTERM, SIGINT])?;
    let stopper = Arc::clone(&server);
    std::thread::spawn(move || {
        if let Some(sig) = signals.forever().next() {
            eprintln!("signal {sig}: no new requests; draining");
            stopper.unblock();
        }
    });
    let running = serve_on(&server, &Arc::new(cfg));
    let left = drain(running, DRAIN_DEADLINE);
    if left > 0 {
        eprintln!(
            "stopping with {left} request(s) still running; their callers must treat a sign as unknown"
        );
    }
    eprintln!("stopped");
    Ok(())
}

/// One thread per request; signing blocks while it confirms. Returns the threads still running
/// when the accept loop ends.
fn serve_on(server: &tiny_http::Server, cfg: &Arc<Config>) -> Vec<JoinHandle<()>> {
    let mut running: Vec<JoinHandle<()>> = Vec::new();
    for request in server.incoming_requests() {
        running.retain(|h| !h.is_finished());
        if running.len() >= MAX_IN_FLIGHT {
            let busy =
                Response::from_string(r#"{"error":"busy; retry shortly"}"#).with_status_code(503);
            let _ = request.respond(busy);
            continue;
        }
        let cfg = Arc::clone(cfg);
        running.push(std::thread::spawn(move || handle(&cfg, request)));
    }
    running
}

/// Waits for running requests until `deadline`; returns how many were still running.
fn drain(mut running: Vec<JoinHandle<()>>, deadline: Duration) -> usize {
    let start = Instant::now();
    loop {
        running.retain(|h| !h.is_finished());
        if running.is_empty() || start.elapsed() >= deadline {
            return running.len();
        }
        std::thread::sleep(DRAIN_POLL);
    }
}

struct Reply {
    status: u16,
    body: Option<Value>,
}

const fn reply(status: u16, body: Value) -> Reply {
    Reply {
        status,
        body: Some(body),
    }
}

fn handle(cfg: &Config, mut request: Request) {
    let path = request
        .url()
        .split('?')
        .next()
        .unwrap_or_default()
        .to_owned();
    let method = request.method().clone();
    let span = tracing::info_span!(
        "request",
        method = %method,
        path = %path,
        token = tracing::field::Empty,
        status = tracing::field::Empty
    )
    .entered();
    let (r, log) = route(cfg, &mut request, &method, &path);
    span.record("status", r.status);
    if let Some(line) = access_line(crate::trace::active(), &method, &path, r.status, &log) {
        eprintln!("{line}");
    }
    let text = r.body.map(|b| b.to_string()).unwrap_or_default();
    let mut response = Response::from_string(text).with_status_code(r.status);
    if let Ok(h) = "Content-Type: application/json".parse::<Header>() {
        response.add_header(h);
    }
    let _ = request.respond(response);
}

/// The plain access line, only while tracing is off: with it on, the `request` span carries the
/// same fields, and a plain line would break a JSON log.
fn access_line(
    traced: bool,
    method: &Method,
    path: &str,
    status: u16,
    note: &str,
) -> Option<String> {
    (!traced).then(|| format!("{method} {path} -> {status} {note}"))
}

/// Returns the reply and a short log note (never secrets).
fn route(cfg: &Config, request: &mut Request, method: &Method, path: &str) -> (Reply, String) {
    let Some(serve) = cfg.serve.as_ref() else {
        return (reply(500, json!({"error": "not serving"})), String::new());
    };
    if let Err(e) = check_origin(serve, request) {
        return (reply(403, json!({"error": e})), String::new());
    }
    if (method, path) == (&Method::Get, "/health") {
        return (reply(200, json!({"status": "ok"})), String::new());
    }
    let Some(token) = authenticate(serve, request) else {
        return (
            reply(401, json!({"error": "missing or unknown bearer token"})),
            String::new(),
        );
    };
    tracing::Span::current().record("token", token.name.as_str());
    let note = format!("token={}", token.name);
    if method != &Method::Post {
        return (reply(405, json!({"error": "use POST"})), note);
    }
    let body = match read_body(request) {
        Ok(b) => b,
        Err(r) => return (r, note),
    };
    let r = match path {
        "/v1/check" => http_engine(cfg, token, &body, false),
        "/v1/sign" => http_engine(cfg, token, &body, true),
        "/v1/halt" => http_halt(cfg, token, &body),
        "/mcp" => mcp::handle(cfg, token, &body),
        _ => reply(404, json!({"error": "not found"})),
    };
    (r, note)
}

/// Host and Origin must name an allowed host, so a web page cannot reach the signer (DNS rebinding).
fn check_origin(serve: &ServeConfig, request: &Request) -> Result<(), String> {
    let allowed = |h: &str| {
        serve
            .allowed_hosts
            .iter()
            .any(|a| a.eq_ignore_ascii_case(h))
    };
    let host = header(request, "Host").ok_or("missing Host header")?;
    if !allowed(host_name(&host)) {
        return Err(format!("host {host:?} is not allowed"));
    }
    if let Some(origin) = header(request, "Origin") {
        let authority = origin.split("://").nth(1).unwrap_or_default();
        if !allowed(host_name(authority)) {
            return Err(format!("origin {origin:?} is not allowed"));
        }
    }
    Ok(())
}

fn header(request: &Request, name: &'static str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str().to_owned())
}

fn authenticate<'a>(serve: &'a ServeConfig, request: &Request) -> Option<&'a TokenConfig> {
    let value = Zeroizing::new(header(request, "Authorization")?);
    let token = value.strip_prefix("Bearer ")?;
    token_for(serve, token)
}

pub fn token_hash(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

fn token_for<'a>(serve: &'a ServeConfig, token: &str) -> Option<&'a TokenConfig> {
    let hash = token_hash(token);
    // Compare against every token, without short-circuiting on the first differing byte.
    let mut found = None;
    for t in &serve.tokens {
        if constant_time_eq(t.sha256.as_bytes(), hash.as_bytes()) && found.is_none() {
            found = Some(t);
        }
    }
    found
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0_u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn read_body(request: &mut Request) -> Result<Zeroizing<String>, Reply> {
    let mut body = Zeroizing::new(String::new());
    request
        .as_reader()
        .take(MAX_BODY.saturating_add(1))
        .read_to_string(&mut body)
        .map_err(|e| reply(400, json!({"error": format!("reading the body: {e}")})))?;
    if u64::try_from(body.len()).unwrap_or(u64::MAX) > MAX_BODY {
        return Err(reply(413, json!({"error": "body too large"})));
    }
    Ok(body)
}

/// Arguments shared by the HTTP endpoints and the MCP tools.
fn engine_call(
    cfg: &Config,
    token: &TokenConfig,
    args: &Value,
    sign: bool,
) -> Result<engine::Report> {
    let Some(payload) = args.get("payload").filter(|p| p.is_object()) else {
        bail!("`payload` must be the payload object from a forge-mcp build_* call");
    };
    let text = Zeroizing::new(serde_json::to_string(payload)?);
    let key = args.get("key").and_then(Value::as_str);
    let mode = if sign {
        Mode::Sign {
            intent: args
                .get("intent")
                .and_then(Value::as_str)
                .map(str::to_owned),
        }
    } else {
        Mode::Check
    };
    engine::execute(cfg, key, &text, &mode, Some(&token.keys)).map(|(report, _, _)| report)
}

fn http_engine(cfg: &Config, token: &TokenConfig, body: &str, sign: bool) -> Reply {
    let args: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return reply(400, json!({"error": format!("body is not JSON: {e}")})),
    };
    match engine_call(cfg, token, &args, sign) {
        Ok(report) => reply(200, serde_json::to_value(report).unwrap_or_default()),
        Err(e) => reply(500, json!({"error": format!("{e:#}")})),
    }
}

/// Halts `key`, or every key the token lists, with `reason`. Returns the keys halted, or an HTTP
/// status and message. It does not wait for the key lock: a sign already running finishes.
fn halt_keys(
    cfg: &Config,
    token: &TokenConfig,
    args: &Value,
) -> Result<Vec<String>, (u16, String)> {
    let reason = args
        .get("reason")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .ok_or_else(|| (400, "`reason` is required".to_owned()))?;
    if reason.chars().count() > MAX_REASON {
        return Err((400, format!("`reason` is over {MAX_REASON} characters")));
    }
    let keys = match args.get("key") {
        None | Some(Value::Null) => token.keys.clone(),
        Some(Value::String(k)) if token.keys.contains(k) => vec![k.clone()],
        Some(Value::String(k)) => {
            return Err((403, format!("this token may not use key {k:?}")));
        }
        Some(_) => return Err((400, "`key` must be a string".to_owned())),
    };
    let h = Halt {
        ts: engine::now(),
        reason: reason.to_owned(),
        by: format!("token {}", token.name),
    };
    for k in &keys {
        halt::set(&cfg.state_dir, Some(k), &h).map_err(|e| (500, format!("{e:#}")))?;
    }
    Ok(keys)
}

fn http_halt(cfg: &Config, token: &TokenConfig, body: &str) -> Reply {
    let args: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return reply(400, json!({"error": format!("body is not JSON: {e}")})),
    };
    match halt_keys(cfg, token, &args) {
        Ok(keys) => reply(200, json!({"halted": keys})),
        Err((status, e)) => reply(status, json!({"error": e})),
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn constant_time_eq_matches_only_equal_bytes() {
        assert!(super::constant_time_eq(b"abcd", b"abcd"));
        assert!(!super::constant_time_eq(b"abcd", b"abce"));
        assert!(!super::constant_time_eq(b"abcd", b"abc"));
    }

    use super::*;

    fn serve() -> ServeConfig {
        ServeConfig {
            listen: String::new(),
            allowed_hosts: vec!["127.0.0.1".into(), "localhost".into()],
            tokens: vec![TokenConfig {
                name: "t".into(),
                sha256: token_hash("secret"),
                keys: vec!["k".into()],
            }],
        }
    }

    #[test]
    fn tokens_match_by_hash_only() {
        let s = serve();
        assert_eq!(token_for(&s, "secret").map(|t| t.name.as_str()), Some("t"));
        assert!(token_for(&s, "Secret").is_none());
        assert!(token_for(&s, "").is_none());
    }

    /// A live server on a random port, with keys `mine` (in the token) and `other` (not).
    fn live(tag: &str) -> (String, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "sa-forge-signer-serve-{tag}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg_path = dir.join("config.toml");
        std::fs::write(
            &cfg_path,
            format!(
                "[keys.mine]\nclass = \"session\"\npubkey = \"{MINE}\"\n[keys.other]\nclass = \"session\"\npubkey = \"{OTHER}\"\n\
                 [serve]\nlisten = \"127.0.0.1:0\"\n[[serve.tokens]]\nname = \"t\"\nsha256 = \"{}\"\nkeys = [\"mine\"]\n",
                token_hash("secret")
            ),
        )
        .unwrap();
        let cfg = Arc::new(Config::load(&cfg_path, true).unwrap());
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || serve_on(&server, &cfg));
        (format!("http://127.0.0.1:{port}"), dir)
    }

    const MINE: &str = "7JJhMrnvgqxS6MW59QQcr2Ki1NjZ7SrAjST4YtAFE8zR";
    const OTHER: &str = "2vFWMdAXYjEXe1ounwAgm4gP9z4uLBPqjh1P8eDH7gFH";

    fn post(url: &str, token: Option<&str>, host: Option<&str>, body: &Value) -> (u16, String) {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let mut req = agent.post(url);
        if let Some(t) = token {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        if let Some(h) = host {
            req = req.header("Host", h);
        }
        let mut resp = req.send_json(body).unwrap();
        (
            resp.status().as_u16(),
            resp.body_mut().read_to_string().unwrap(),
        )
    }

    #[test]
    fn serves_health_auth_and_mcp_over_http() {
        let (base, dir) = live("mcp");
        let mut health = ureq::get(&format!("{base}/health")).call().unwrap();
        assert_eq!(
            health.body_mut().read_to_string().unwrap(),
            r#"{"status":"ok"}"#
        );

        let mcp = format!("{base}/mcp");
        let init = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}});
        assert_eq!(post(&mcp, None, None, &init).0, 401);
        assert_eq!(post(&mcp, Some("wrong"), None, &init).0, 401);
        assert_eq!(
            post(&mcp, Some("secret"), Some("evil.example"), &init).0,
            403
        );

        let (status, body) = post(&mcp, Some("secret"), None, &init);
        assert_eq!(status, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(v["result"]["serverInfo"]["name"], "sa-forge-signer");

        let note = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        assert_eq!(post(&mcp, Some("secret"), None, &note).0, 202);

        let list = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
        let v: Value = serde_json::from_str(&post(&mcp, Some("secret"), None, &list).1).unwrap();
        assert_eq!(v["result"]["tools"].as_array().unwrap().len(), 4);

        let unknown = json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"});
        let v: Value = serde_json::from_str(&post(&mcp, Some("secret"), None, &unknown).1).unwrap();
        assert_eq!(v["error"]["code"], -32601);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_stdio_proxy_reaches_the_same_tools() {
        let (base, dir) = live("stdio");
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n",
        );
        let out = crate::mcp_cli::tests::piped(input, crate::mcp_cli::tests::live(&base, "secret"));
        assert_eq!(out.len(), 2, "{out:?}");
        assert_eq!(out[0]["result"]["serverInfo"]["name"], "sa-forge-signer");
        assert_eq!(out[1]["result"]["tools"].as_array().unwrap().len(), 4);
        let denied =
            crate::mcp_cli::tests::piped(input, crate::mcp_cli::tests::live(&base, "wrong"));
        assert!(
            denied[0]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("bearer token"),
            "{denied:?}"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_token_cannot_use_another_key() {
        let (base, dir) = live("auth");
        let payload = json!({"version": 1, "summary": "s", "transaction": {"fee_payer": OTHER, "instructions": [
            {"program_id": "ComputeBudget111111111111111111111111111111", "accounts": [], "data": [2, 64, 66, 15, 0]}]}});
        let (status, body) = post(
            &format!("{base}/v1/check"),
            Some("secret"),
            None,
            &json!({"payload": payload}),
        );
        assert_eq!(status, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["outcome"], "refused");
        assert_eq!(v["failed_check"], "authorized");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn drain_waits_for_running_requests_up_to_the_deadline() {
        let quick = std::thread::spawn(|| std::thread::sleep(Duration::from_millis(50)));
        assert_eq!(drain(vec![quick], Duration::from_secs(5)), 0);
        let slow = std::thread::spawn(|| std::thread::sleep(Duration::from_secs(3)));
        let start = Instant::now();
        assert_eq!(drain(vec![slow], Duration::from_millis(200)), 1);
        assert!(start.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn unblock_ends_the_accept_loop() {
        let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").unwrap());
        let stopper = Arc::clone(&server);
        let cfg = Arc::new(
            Config::load(std::path::Path::new("/nonexistent/config.toml"), false).unwrap(),
        );
        let loop_thread = std::thread::spawn(move || serve_on(&server, &cfg).len());
        std::thread::sleep(Duration::from_millis(100));
        stopper.unblock();
        assert_eq!(loop_thread.join().unwrap(), 0);
    }

    /// A live server over the mock node, with token `secret` for key `k` only.
    fn live_signing(tag: &str) -> (String, crate::testrpc::Home) {
        let home = crate::testrpc::Home::new(
            tag,
            &format!(
                "[keys.other]\nclass = \"session\"\npubkey = \"{OTHER}\"\n[serve]\n[[serve.tokens]]\nname = \"t\"\nsha256 = \"{}\"\nkeys = [\"k\"]\n",
                token_hash("secret")
            ),
        );
        let cfg = Arc::new(Config::load(&home.path, true).unwrap());
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || serve_on(&server, &cfg));
        (format!("http://127.0.0.1:{port}"), home)
    }

    #[test]
    fn a_running_serve_sees_a_cli_halt_on_its_next_request() {
        let (base, home) = live_signing("halt-cli");
        let sign = || {
            let body = json!({"payload": serde_json::from_str::<Value>(&home.payload(1)).unwrap()});
            let (status, text) = post(&format!("{base}/v1/sign"), Some("secret"), None, &body);
            assert_eq!(status, 200, "{text}");
            serde_json::from_str::<Value>(&text).unwrap()
        };
        assert_eq!(sign()["outcome"], "confirmed");
        let halt = crate::halt::Halt {
            ts: 1,
            reason: "operator stop".into(),
            by: "cli".into(),
        };
        crate::halt::set(&home.cfg.state_dir, Some("k"), &halt).unwrap();
        let r = sign();
        assert_eq!(
            (&r["outcome"], &r["failed_check"]),
            (&json!("refused"), &json!("halt"))
        );
        assert_eq!(home.node.sends(), 1);
    }

    #[test]
    fn another_tokens_refused_signs_do_not_use_up_a_keys_rate() {
        // Key k signs at most 2 a minute; token "agent" may use it, token "intruder" may not.
        let home = crate::testrpc::Home::new(
            "rate-tokens",
            &format!(
                "rate_limit_per_minute = 2\n[keys.other]\nclass = \"session\"\npubkey = \"{OTHER}\"\n[serve]\n\
                 [[serve.tokens]]\nname = \"agent\"\nsha256 = \"{}\"\nkeys = [\"k\"]\n\
                 [[serve.tokens]]\nname = \"intruder\"\nsha256 = \"{}\"\nkeys = [\"other\"]\n",
                token_hash("agent-secret"),
                token_hash("intruder-secret")
            ),
        );
        let cfg = Arc::new(Config::load(&home.path, true).unwrap());
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || serve_on(&server, &cfg));
        let base = format!("http://127.0.0.1:{port}");
        // Each sign moves a different amount, so none is a duplicate of another.
        let sign = |token: &str, lamports: u64| {
            let body = json!({"key": "k", "payload": serde_json::from_str::<Value>(&home.payload(lamports)).unwrap()});
            let (status, text) = post(&format!("{base}/v1/sign"), Some(token), None, &body);
            assert_eq!(status, 200, "{text}");
            serde_json::from_str::<Value>(&text).unwrap()
        };
        for n in 1..=5 {
            assert_eq!(sign("intruder-secret", n)["failed_check"], "authorized");
        }
        assert_eq!(sign("agent-secret", 6)["outcome"], "confirmed");
        assert_eq!(sign("agent-secret", 7)["outcome"], "confirmed");
        // The key's own two signs use up its rate; the refusals before them did not.
        let r = sign("agent-secret", 8);
        assert_eq!(
            (&r["outcome"], &r["failed_check"]),
            (&json!("refused"), &json!("limits"))
        );
        assert_eq!(home.node.sends(), 2);
    }

    #[test]
    fn a_token_can_halt_its_own_keys_but_never_resume() {
        let (base, home) = live_signing("halt-token");
        let halt = format!("{base}/v1/halt");
        let (status, _) = post(
            &halt,
            Some("secret"),
            None,
            &json!({"key": "other", "reason": "x"}),
        );
        assert_eq!(status, 403, "a key the token does not list");
        assert_eq!(
            post(&halt, Some("secret"), None, &json!({"key": "k"})).0,
            400,
            "no reason"
        );
        assert!(
            crate::halt::active(&home.cfg.state_dir, "k")
                .unwrap()
                .is_none()
        );
        let (status, body) = post(
            &halt,
            Some("secret"),
            None,
            &json!({"reason": "misbehaving"}),
        );
        assert_eq!(status, 200, "{body}");
        assert_eq!(
            serde_json::from_str::<Value>(&body).unwrap()["halted"],
            json!(["k"])
        );
        let h = crate::halt::active(&home.cfg.state_dir, "k")
            .unwrap()
            .unwrap();
        assert_eq!(
            (h.reason.as_str(), h.by.as_str()),
            ("misbehaving", "token t")
        );
        assert!(
            crate::halt::active(&home.cfg.state_dir, "other")
                .unwrap()
                .is_none()
        );
        let (status, _) = post(
            &format!("{base}/v1/resume"),
            Some("secret"),
            None,
            &json!({"key": "k"}),
        );
        assert_eq!(status, 404);

        let call = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "halt", "arguments": {"key": "k", "reason": "again"}}});
        let v: Value =
            serde_json::from_str(&post(&format!("{base}/mcp"), Some("secret"), None, &call).1)
                .unwrap();
        assert_eq!(v["result"]["isError"], false, "{v}");
        let denied = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "halt", "arguments": {"key": "other", "reason": "x"}}});
        let v: Value =
            serde_json::from_str(&post(&format!("{base}/mcp"), Some("secret"), None, &denied).1)
                .unwrap();
        assert_eq!(v["result"]["isError"], true);
        let resume = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
            "params": {"name": "resume", "arguments": {"key": "k"}}});
        let v: Value =
            serde_json::from_str(&post(&format!("{base}/mcp"), Some("secret"), None, &resume).1)
                .unwrap();
        assert_eq!(v["error"]["code"], -32602, "there is no resume tool");
        assert!(
            crate::halt::active(&home.cfg.state_dir, "k")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn key_show_says_whether_the_key_is_halted_or_awaiting_approval() {
        let home = crate::testrpc::Home::new(
            "key-show",
            &format!(
                "require_policy_approval = true\n[serve]\n[[serve.tokens]]\nname = \"t\"\nsha256 = \"{}\"\nkeys = [\"k\"]\n",
                token_hash("secret")
            ),
        );
        let cfg = &home.cfg;
        let token = &cfg.serve.as_ref().unwrap().tokens[0];
        let show = || {
            let (text, err) = mcp::key_show(cfg, token, &json!({"name": "k"}));
            assert!(!err, "{text}");
            serde_json::from_str::<Value>(&text).unwrap()
        };
        let digest = cfg.key("k").unwrap().policy.clone();
        let v = show();
        assert_eq!(v["halted"], Value::Null);
        assert_eq!(
            v["policy"],
            json!({"digest": digest, "approval_required": true, "approved": false})
        );
        crate::policy_approval::approve(
            &cfg.state_dir,
            cfg.key("k").unwrap(),
            Some(crate::halt::tests::term("yes\n")),
            1,
        )
        .unwrap();
        crate::halt::set(
            &cfg.state_dir,
            None,
            &crate::halt::Halt {
                ts: 2,
                reason: "stop".into(),
                by: "cli".into(),
            },
        )
        .unwrap();
        let v = show();
        assert_eq!(v["policy"]["approved"], true);
        assert_eq!(
            v["halted"],
            json!({"since": 2, "by": "cli", "reason": "stop"})
        );
        std::fs::write(cfg.state_dir.join("halts/all.json"), "{").unwrap();
        assert!(
            mcp::key_show(cfg, token, &json!({"name": "k"})).1,
            "an unreadable halt is an error"
        );
    }

    #[test]
    fn the_plain_access_line_is_only_for_untraced_runs() {
        assert_eq!(
            access_line(false, &Method::Post, "/v1/sign", 200, "token=agent").as_deref(),
            Some("POST /v1/sign -> 200 token=agent")
        );
        assert_eq!(
            access_line(true, &Method::Post, "/v1/sign", 200, "token=agent"),
            None
        );
    }
}
