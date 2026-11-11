//! What the service and the stdio relay share of MCP revision 2026-07-28: the names of its request
//! metadata and HTTP headers, its error codes, and how a header value mirrors the body.

use base64::Engine;
use serde_json::Value;

/// The only protocol revision the signer speaks.
pub const VERSION: &str = "2026-07-28";

pub const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
pub const META_SERVER_INFO: &str = "io.modelcontextprotocol/serverInfo";

pub const HEADER_VERSION: &str = "MCP-Protocol-Version";
pub const HEADER_METHOD: &str = "Mcp-Method";
pub const HEADER_NAME: &str = "Mcp-Name";

pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;
pub const HEADER_MISMATCH: i64 = -32020;
pub const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;

const BASE64_OPEN: &str = "=?base64?";
const BASE64_CLOSE: &str = "?=";

/// The protocol version a request declares, in its `params._meta`.
pub fn version_of(params: &Value) -> Option<&str> {
    params.get("_meta")?.get(META_VERSION)?.as_str()
}

/// The `params` field the `Mcp-Name` header mirrors for `method`, or `None` for a method with no name.
pub fn name_field(method: &str) -> Option<&'static str> {
    match method {
        "tools/call" | "prompts/get" => Some("name"),
        "resources/read" => Some("uri"),
        _ => None,
    }
}

/// A body value as a header carries it: as it is when it is plain ASCII, and base64 otherwise.
pub fn encode_header(value: &str) -> String {
    let plain = value.chars().all(|c| (' '..='~').contains(&c))
        && value.trim() == value
        && !value.starts_with(BASE64_OPEN);
    if plain {
        value.to_owned()
    } else {
        let encoded = base64::engine::general_purpose::STANDARD.encode(value);
        format!("{BASE64_OPEN}{encoded}{BASE64_CLOSE}")
    }
}

/// The body value a header stands for, or `None` when its base64 does not decode.
pub fn decode_header(value: &str) -> Option<String> {
    let Some(encoded) = value
        .strip_prefix(BASE64_OPEN)
        .and_then(|rest| rest.strip_suffix(BASE64_CLOSE))
    else {
        return Some(value.to_owned());
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    String::from_utf8(bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_values_pass_and_the_rest_are_base64() {
        for plain in ["get_weather", "sa-forge-signer://llms.txt", "a b", ""] {
            assert_eq!(encode_header(plain), plain);
            assert_eq!(decode_header(plain).as_deref(), Some(plain));
        }
        // The specification's own example.
        assert_eq!(
            encode_header("Hello, 世界"),
            "=?base64?SGVsbG8sIOS4lueVjA==?="
        );
        for odd in [
            "Hello, 世界",
            " lead",
            "trail ",
            "line\nbreak",
            "=?base64?x?=",
        ] {
            let sent = encode_header(odd);
            assert!(sent.starts_with(BASE64_OPEN), "{sent}");
            assert_eq!(decode_header(&sent).as_deref(), Some(odd));
        }
        assert_eq!(decode_header("=?base64?!!!?="), None);
    }

    #[test]
    fn the_name_header_mirrors_the_tool_or_the_uri() {
        assert_eq!(name_field("tools/call"), Some("name"));
        assert_eq!(name_field("resources/read"), Some("uri"));
        assert_eq!(name_field("tools/list"), None);
        let params = json!({"_meta": {META_VERSION: VERSION}});
        assert_eq!(version_of(&params), Some(VERSION));
        assert_eq!(version_of(&json!({})), None);
    }
}
