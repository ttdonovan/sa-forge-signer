//! The `sa-forge-signer-mcp` command line: MCP on standard input and output.
//!
//! Every message is forwarded to a running `sa-forge-signer-serve`. This process holds a URL and a
//! bearer token, never a key or the config: an MCP client starts it as the agent's own OS user,
//! which must not be able to read either.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Parser;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::url;

/// Longer than the service's own drain of a running sign (220 s), so the service answers first.
const TIMEOUT: Duration = Duration::from_secs(240);

/// MCP over standard input and output for a running sa-forge-signer-serve: the same tools as its /mcp route
#[derive(Parser)]
#[command(name = "sa-forge-signer-mcp", version, long_about = None)]
struct Cli {
    /// Base URL of the running service; https, or http only to a loopback host
    #[arg(long, default_value = "http://127.0.0.1:8790")]
    url: String,
    /// File holding the bearer token (from `sa-forge-signer token new`)
    #[arg(long)]
    token_file: PathBuf,
}

/// Forwards until standard input closes and returns the exit code.
#[must_use]
pub fn main() -> ExitCode {
    match run(&Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<()> {
    let endpoint = endpoint(&cli.url)?;
    let token = read_token(&cli.token_file)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let post = |body: &str| post(&agent, &endpoint, &token, body);
    // Standard output carries the protocol only.
    pump(std::io::stdin().lock(), std::io::stdout().lock(), post)
}

/// The service's MCP route under `url`, which must not carry the token across a network in clear.
fn endpoint(base: &str) -> Result<String> {
    if !url::secure(base) {
        bail!("--url: must start with https://, or http:// only for a loopback host");
    }
    Ok(format!("{}/mcp", base.trim_end_matches('/')))
}

fn read_token(path: &Path) -> Result<Zeroizing<String>> {
    let text = Zeroizing::new(
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
    );
    let token = Zeroizing::new(text.trim().to_owned());
    if token.is_empty() {
        bail!("{} is empty", path.display());
    }
    Ok(token)
}

/// One request to the service: its HTTP status and body, or why it could not be reached.
fn post(
    agent: &ureq::Agent,
    endpoint: &str,
    token: &str,
    body: &str,
) -> std::result::Result<(u16, String), String> {
    let mut resp = agent
        .post(endpoint)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .send(body)
        .map_err(|e| e.to_string())?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("HTTP {status}: {e}"))?;
    Ok((status, text))
}

/// Forwards each line of `input` (one JSON-RPC message) and writes each reply as one line.
fn pump(
    input: impl BufRead,
    mut output: impl Write,
    post: impl Fn(&str) -> std::result::Result<(u16, String), String>,
) -> Result<()> {
    for line in input.lines() {
        // A payload may carry partial-signer secrets.
        let line = Zeroizing::new(line.context("reading standard input")?);
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = forward(&line, &post) {
            writeln!(output, "{reply}").context("writing standard output")?;
            output.flush().context("writing standard output")?;
        }
    }
    Ok(())
}

/// The reply to one message, or `None` when it gets none (a notification). The service's JSON-RPC
/// replies pass through unchanged; anything else becomes a JSON-RPC error for the request's `id`.
fn forward(
    line: &str,
    post: impl Fn(&str) -> std::result::Result<(u16, String), String>,
) -> Option<String> {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return Some(error(&Value::Null, -32700, "parse error"));
    };
    let id = message.get("id").cloned();
    let failed = |what: String| {
        id.as_ref().map(|id| {
            error(
                id,
                -32000,
                &format!(
                    "{what}. If this was a `sign`, its outcome is unknown: the transaction may still have landed, so look it up and re-read the game state before building it again"
                ),
            )
        })
    };
    match post(line) {
        Ok((_, body)) => match serde_json::from_str::<Value>(&body) {
            // One line, whatever the service's formatting.
            Ok(reply) if reply.get("jsonrpc").is_some() => Some(reply.to_string()),
            Ok(reply) => {
                let said = reply.get("error").and_then(Value::as_str).unwrap_or(&body);
                id.as_ref().map(|id| {
                    error(
                        id,
                        -32000,
                        &format!("the signer refused the request: {said}"),
                    )
                })
            }
            Err(_) if body.trim().is_empty() && id.is_none() => None,
            Err(_) => failed("the signer's reply was not JSON".to_owned()),
        },
        Err(e) => failed(format!("could not reach the signer: {e}")),
    }
}

fn error(id: &Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use clap::CommandFactory;

    type Posted = std::result::Result<(u16, String), String>;

    /// Runs `input` through the proxy with `post` as the service; returns what it wrote.
    pub fn piped(input: &str, post: impl Fn(&str) -> Posted) -> Vec<Value> {
        let mut out = Vec::new();
        pump(input.as_bytes(), &mut out, post).unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// A poster for a live service at `base`.
    pub fn live(base: &str, token: &str) -> impl Fn(&str) -> Posted {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let endpoint = endpoint(base).unwrap();
        let token = token.to_owned();
        move |body: &str| post(&agent, &endpoint, &token, body)
    }

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
        assert!(Cli::try_parse_from(["sa-forge-signer-mcp", "--token-file", "t"]).is_ok());
        assert!(Cli::try_parse_from(["sa-forge-signer-mcp"]).is_err());
    }

    #[test]
    fn the_url_must_not_carry_the_token_in_clear() {
        assert_eq!(
            endpoint("http://127.0.0.1:8790/").unwrap(),
            "http://127.0.0.1:8790/mcp"
        );
        assert_eq!(
            endpoint("https://signer.example").unwrap(),
            "https://signer.example/mcp"
        );
        assert!(endpoint("http://signer.example:8790").is_err());
    }

    #[test]
    fn replies_pass_through_one_per_line_and_notifications_get_none() {
        let service = |body: &str| -> Posted {
            let m: Value = serde_json::from_str(body).unwrap();
            m.get("id").map_or_else(
                || Ok((202, String::new())),
                |id| {
                    Ok((
                        200,
                        format!("{{\n \"jsonrpc\": \"2.0\", \"id\": {id}, \"result\": {{}}\n}}"),
                    ))
                },
            )
        };
        let out = piped(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n{\"jsonrpc\":\"2.0\",\"id\":\"b\",\"method\":\"ping\"}\n",
            service,
        );
        assert_eq!(
            out,
            [
                json!({"jsonrpc": "2.0", "id": 1, "result": {}}),
                json!({"jsonrpc": "2.0", "id": "b", "result": {}})
            ]
        );
    }

    #[test]
    fn a_refusal_or_a_lost_connection_becomes_an_error_for_that_request() {
        let call = "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/call\"}\n";
        let denied = piped(call, |_| {
            Ok((
                401,
                r#"{"error":"missing or unknown bearer token"}"#.to_owned(),
            ))
        });
        assert_eq!(denied[0]["id"], 7);
        assert!(
            denied[0]["error"]["message"]
                .as_str()
                .unwrap()
                .contains("missing or unknown bearer token")
        );
        let lost = piped(call, |_| Err("connection reset".to_owned()));
        let message = lost[0]["error"]["message"].as_str().unwrap();
        assert!(message.contains("connection reset") && message.contains("unknown"));
        // Nothing to answer when a notification fails; a line that is not JSON has no id.
        assert!(
            piped("{\"jsonrpc\":\"2.0\",\"method\":\"x\"}\n", |_| Err(
                "down".to_owned()
            ))
            .is_empty()
        );
        assert_eq!(
            piped("not json\n", |_| Err("unused".to_owned()))[0]["error"]["code"],
            -32700
        );
    }

    #[test]
    fn a_token_file_is_trimmed_and_must_not_be_empty() {
        let dir = std::env::temp_dir().join(format!("sa-forge-signer-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t");
        std::fs::write(&path, "secret\n").unwrap();
        assert_eq!(read_token(&path).unwrap().as_str(), "secret");
        std::fs::write(&path, "\n").unwrap();
        assert!(read_token(&path).is_err());
        assert!(read_token(&dir.join("missing")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
