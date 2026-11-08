//! The RPC calls the page makes, and the network it accepts.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use crate::bridge::{rpc, sleep};
use crate::profile::{FIRST_KEY_OFFSET, KEY_LEN, PLAYER_PROFILE_PROGRAM, PROFILE_DISCRIMINATOR};
use crate::tx::{Pubkey, parse_pubkey};

/// Z.ink testnet, as the signer's `zink-testnet` cluster profile.
pub const DEFAULT_RPC: &str = "https://rpc1.z.ink";
pub const GENESIS_HASH: &str = "6qaAozzun2WV83PRDgqf79WXtbqfnezKB3demjxXV5EY";
pub const EXPLORER: &str = "https://explorer.z.ink";

/// Profile key positions searched for the wallet; the authority is usually at 0.
const SEARCH_POSITIONS: usize = 4;

fn str_at<'a>(v: &'a Value, path: &str) -> Result<&'a str, String> {
    v.pointer(path)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("RPC response has no {path}"))
}

pub async fn check_network(url: &str) -> Result<(), String> {
    let hash = rpc(url, "getGenesisHash", &json!([])).await?;
    match hash.as_str() {
        Some(GENESIS_HASH) => Ok(()),
        Some(other) => Err(format!(
            "this RPC serves another network (genesis {other}); the page only works on Z.ink testnet"
        )),
        None => Err("getGenesisHash returned no hash".into()),
    }
}

pub async fn latest_blockhash(url: &str) -> Result<Pubkey, String> {
    let r = rpc(
        url,
        "getLatestBlockhash",
        &json!([{ "commitment": "confirmed" }]),
    )
    .await?;
    parse_pubkey(str_at(&r, "/value/blockhash")?)
}

pub async fn account_data(url: &str, address: &str) -> Result<Vec<u8>, String> {
    let r = rpc(
        url,
        "getAccountInfo",
        &json!([address, { "encoding": "base64", "commitment": "confirmed" }]),
    )
    .await?;
    let value = r.get("value").ok_or("getAccountInfo returned no value")?;
    if value.is_null() {
        return Err(format!("no account at {address}"));
    }
    let owner = str_at(value, "/owner")?;
    if owner != PLAYER_PROFILE_PROGRAM {
        return Err(format!(
            "{address} is not owned by the Player Profile program"
        ));
    }
    STANDARD
        .decode(str_at(value, "/data/0")?)
        .map_err(|e| format!("account data: {e}"))
}

/// Profiles holding `wallet` as one of their first keys.
pub async fn find_profiles(url: &str, wallet: &str) -> Result<Vec<String>, String> {
    let disc = bs58::encode(PROFILE_DISCRIMINATOR).into_string();
    let mut found: Vec<String> = Vec::new();
    for position in 0..SEARCH_POSITIONS {
        let offset = position
            .checked_mul(KEY_LEN)
            .and_then(|o| o.checked_add(FIRST_KEY_OFFSET))
            .ok_or("offset overflow")?;
        let r = rpc(
            url,
            "getProgramAccounts",
            &json!([PLAYER_PROFILE_PROGRAM, {
                "encoding": "base64",
                "dataSlice": { "offset": 0, "length": 0 },
                "filters": [
                    { "memcmp": { "offset": 0, "bytes": disc } },
                    { "memcmp": { "offset": offset, "bytes": wallet } },
                ],
            }]),
        )
        .await?;
        for acc in r.as_array().ok_or("getProgramAccounts returned no list")? {
            let key = str_at(acc, "/pubkey")?.to_string();
            if !found.contains(&key) {
                found.push(key);
            }
        }
    }
    Ok(found)
}

/// Simulates without signatures; returns the logs, or the error with them.
pub async fn simulate(url: &str, tx_base64: &str) -> Result<Vec<String>, String> {
    let r = rpc(
        url,
        "simulateTransaction",
        &json!([tx_base64, {
            "encoding": "base64",
            "sigVerify": false,
            "replaceRecentBlockhash": true,
            "commitment": "confirmed",
        }]),
    )
    .await?;
    let value = r
        .get("value")
        .ok_or("simulateTransaction returned no value")?;
    let logs: Vec<String> = value
        .get("logs")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    match value.get("err") {
        Some(err) if !err.is_null() => {
            Err(format!("simulation failed: {err}\n{}", logs.join("\n")))
        }
        _ => Ok(logs),
    }
}

pub async fn send(url: &str, tx_base64: &str) -> Result<String, String> {
    let r = rpc(
        url,
        "sendTransaction",
        &json!([tx_base64, { "encoding": "base64", "preflightCommitment": "confirmed" }]),
    )
    .await?;
    r.as_str()
        .map(String::from)
        .ok_or_else(|| "sendTransaction returned no signature".into())
}

/// Polls until `signature` is confirmed or fails, for about a minute.
pub async fn confirm(url: &str, signature: &str) -> Result<(), String> {
    for _ in 0..60 {
        let r = rpc(url, "getSignatureStatuses", &json!([[signature]])).await?;
        if let Some(status) = r.pointer("/value/0").filter(|s| !s.is_null()) {
            if let Some(err) = status.get("err").filter(|e| !e.is_null()) {
                return Err(format!("the transaction landed and failed: {err}"));
            }
            if matches!(
                status.get("confirmationStatus").and_then(Value::as_str),
                Some("confirmed" | "finalized")
            ) {
                return Ok(());
            }
        }
        sleep(1_000).await;
    }
    Err(format!(
        "not confirmed after a minute; check {EXPLORER}/tx/{signature} before trying again"
    ))
}
