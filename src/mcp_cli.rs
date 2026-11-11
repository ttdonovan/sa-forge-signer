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
use clap::{Parser, Subcommand};
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::mcp_wire::{
    HEADER_METHOD, HEADER_NAME, HEADER_VERSION, INTERNAL_ERROR, PARSE_ERROR, encode_header,
    name_field, version_of,
};
use crate::{skill, url};

/// Longer than the service's own drain of a running sign (220 s), so the service answers first.
const TIMEOUT: Duration = Duration::from_secs(240);

/// MCP over standard input and output for a running sa-forge-signer-serve: the same tools as its /mcp route
#[derive(Parser)]
#[command(name = "sa-forge-signer-mcp", version, long_about = None)]
struct Cli {
    /// Base URL of the running service; https, or http only to a loopback host
    #[arg(long, default_value = "http://127.0.0.1:8790")]
    url: String,
    /// File holding the bearer token (from `sa-forge-signer token new`); needed to serve
    #[arg(long)]
    token_file: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Forward standard input and output to the service until the client closes it (the default)
    Serve,
    /// The skill that teaches an agent to use the signer's tools
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
}

#[derive(Subcommand)]
enum SkillAction {
    /// Print the skill file
    Print,
    /// Write it as `~/.claude/skills/sa-forge-signer/SKILL.md`, or into a project with --project
    Install {
        /// The project folder whose `.claude/skills` to install into
        #[arg(long)]
        project: Option<PathBuf>,
    },
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
    match &cli.command {
        None | Some(Command::Serve) => serve(cli),
        Some(Command::Skill {
            action: SkillAction::Print,
        }) => {
            print!("{}", skill::text());
            Ok(())
        }
        Some(Command::Skill {
            action: SkillAction::Install { project },
        }) => {
            let home = std::env::var_os("HOME").map(PathBuf::from);
            let skills = skill::skills_dir(project.as_deref(), home.as_deref())?;
            match skill::install(&skills)? {
                skill::Installed::Created(path) => println!("installed {}", path.display()),
                skill::Installed::Replaced(path) => println!("updated {}", path.display()),
                skill::Installed::Unchanged(path) => {
                    println!("already up to date: {}", path.display());
                }
            }
            Ok(())
        }
    }
}

fn serve(cli: &Cli) -> Result<()> {
    let endpoint = endpoint(&cli.url)?;
    let token_file = cli
        .token_file
        .as_deref()
        .context("--token-file is required to serve")?;
    let token = read_token(token_file)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .http_status_as_error(false)
        .build()
        .into();
    let post = |body: &str, headers: &[Header]| post(&agent, &endpoint, &token, body, headers);
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

/// A header the relay adds, mirroring the message it forwards.
type Header = (&'static str, String);

/// One request to the service: its HTTP status and body, or why it could not be reached.
fn post(
    agent: &ureq::Agent,
    endpoint: &str,
    token: &str,
    body: &str,
    headers: &[Header],
) -> std::result::Result<(u16, String), String> {
    let mut request = agent
        .post(endpoint)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream");
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let mut resp = request.send(body).map_err(|e| e.to_string())?;
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
    post: impl Fn(&str, &[Header]) -> std::result::Result<(u16, String), String>,
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

/// The headers the HTTP transport requires, taken from the message: on standard input and output
/// there are none, so the body is all a client sends.
fn mirror(message: &Value) -> Vec<Header> {
    let mut headers = Vec::new();
    let params = message.get("params");
    if let Some(version) = params.and_then(version_of) {
        headers.push((HEADER_VERSION, encode_header(version)));
    }
    let Some(method) = message.get("method").and_then(Value::as_str) else {
        return headers;
    };
    headers.push((HEADER_METHOD, encode_header(method)));
    let name = name_field(method)
        .and_then(|field| params?.get(field))
        .and_then(Value::as_str);
    if let Some(name) = name {
        headers.push((HEADER_NAME, encode_header(name)));
    }
    headers
}

/// The reply to one message, or `None` when it gets none (a notification). The service's JSON-RPC
/// replies pass through unchanged; anything else becomes a JSON-RPC error for the request's `id`.
fn forward(
    line: &str,
    post: impl Fn(&str, &[Header]) -> std::result::Result<(u16, String), String>,
) -> Option<String> {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return Some(error(
            None,
            PARSE_ERROR,
            "Parse error: the line is not JSON",
        ));
    };
    let id = message.get("id");
    let failed = |what: String| {
        id.map(|id| {
            error(
                Some(id),
                INTERNAL_ERROR,
                &format!(
                    "{what}. If this was a `sign`, its outcome is unknown: the transaction may still have landed, so look it up and re-read the game state before building it again"
                ),
            )
        })
    };
    match post(line, &mirror(&message)) {
        Ok((_, body)) => match serde_json::from_str::<Value>(&body) {
            // One line, whatever the service's formatting.
            Ok(reply) if reply.get("jsonrpc").is_some() => Some(reply.to_string()),
            Ok(reply) => {
                let said = reply.get("error").and_then(Value::as_str).unwrap_or(&body);
                id.map(|id| {
                    error(
                        Some(id),
                        INTERNAL_ERROR,
                        &format!("The signer refused the request: {said}"),
                    )
                })
            }
            Err(_) if body.trim().is_empty() && id.is_none() => None,
            Err(_) => failed("The signer's reply was not JSON".to_owned()),
        },
        Err(e) => failed(format!("Could not reach the signer: {e}")),
    }
}

/// An error response; `id` is left out when the message's could not be read.
fn error(id: Option<&Value>, code: i64, message: &str) -> String {
    let mut response = json!({"jsonrpc": "2.0", "error": {"code": code, "message": message}});
    if let (Some(id), Some(fields)) = (id, response.as_object_mut()) {
        fields.insert("id".to_owned(), id.clone());
    }
    response.to_string()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use clap::CommandFactory;

    type Posted = std::result::Result<(u16, String), String>;

    /// Runs `input` through the proxy with `post` as the service; returns what it wrote.
    pub fn piped(input: &str, post: impl Fn(&str, &[Header]) -> Posted) -> Vec<Value> {
        let mut out = Vec::new();
        pump(input.as_bytes(), &mut out, post).unwrap();
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// A poster for a live service at `base`.
    pub fn live(base: &str, token: &str) -> impl Fn(&str, &[Header]) -> Posted {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let endpoint = endpoint(base).unwrap();
        let token = token.to_owned();
        move |body: &str, headers: &[Header]| post(&agent, &endpoint, &token, body, headers)
    }

    #[test]
    fn the_command_line_is_well_formed() {
        Cli::command().debug_assert();
        assert!(Cli::try_parse_from(["sa-forge-signer-mcp", "--token-file", "t"]).is_ok());
        assert!(Cli::try_parse_from(["sa-forge-signer-mcp", "skill", "print"]).is_ok());
        assert!(
            Cli::try_parse_from(["sa-forge-signer-mcp", "skill", "install", "--project", "x"])
                .is_ok()
        );
        assert!(Cli::try_parse_from(["sa-forge-signer-mcp", "skill"]).is_err());
        // Serving needs the token; the skill commands do not.
        let bare = Cli::try_parse_from(["sa-forge-signer-mcp"]).unwrap();
        assert!(run(&bare).is_err());
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
        let service = |body: &str, _: &[Header]| -> Posted {
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
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n\n{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\"}\n{\"jsonrpc\":\"2.0\",\"id\":\"b\",\"method\":\"tools/list\"}\n",
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
        let denied = piped(call, |_, _| {
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
        let lost = piped(call, |_, _| Err("connection reset".to_owned()));
        assert_eq!(lost[0]["error"]["code"], INTERNAL_ERROR);
        let message = lost[0]["error"]["message"].as_str().unwrap();
        assert!(message.contains("connection reset") && message.contains("unknown"));
        // Nothing to answer when a notification fails; a line that is not JSON has no id.
        assert!(
            piped("{\"jsonrpc\":\"2.0\",\"method\":\"x\"}\n", |_, _| Err(
                "down".to_owned()
            ))
            .is_empty()
        );
        let garbled = piped("not json\n", |_, _| Err("unused".to_owned()));
        assert_eq!(garbled[0]["error"]["code"], PARSE_ERROR);
        assert!(garbled[0].get("id").is_none());
    }

    #[test]
    fn the_headers_http_requires_are_taken_from_the_message() {
        let sent = |line: &str| {
            let seen = std::cell::RefCell::new(Vec::new());
            piped(line, |_, headers| {
                seen.borrow_mut().extend(headers.to_vec());
                Ok((202, String::new()))
            });
            seen.into_inner()
        };
        let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"check","_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}}}"#;
        assert_eq!(
            sent(call),
            [
                (HEADER_VERSION, "2026-07-28".to_owned()),
                (HEADER_METHOD, "tools/call".to_owned()),
                (HEADER_NAME, "check".to_owned()),
            ]
        );
        let read =
            r#"{"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"x://世界"}}"#;
        assert_eq!(
            sent(read),
            [
                (HEADER_METHOD, "resources/read".to_owned()),
                (HEADER_NAME, encode_header("x://世界")),
            ]
        );
        // No name for a method that has none; an earlier revision's handshake carries no version.
        let old = r#"{"jsonrpc":"2.0","id":3,"method":"initialize","params":{"protocolVersion":"2025-11-25"}}"#;
        assert_eq!(sent(old), [(HEADER_METHOD, "initialize".to_owned())]);
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
