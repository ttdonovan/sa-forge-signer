//! Tests only: a local JSON-RPC node and a signer config with a temporary key, so whole signs run
//! with no network. Every transaction it is sent "lands" at once with a fixed cost.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use solana_address::Address;
use solana_hash::Hash;

use crate::cluster::ZINK_TESTNET;
use crate::config::Config;
use crate::keystore::KeyStore;

pub const BALANCE: u64 = 1_000_000_000;
pub const SPEND: u64 = 5_000;
const AFTER: u64 = BALANCE - SPEND;

#[derive(Default)]
pub struct Node {
    /// Blockhashes handed out in order; the last repeats.
    pub blockhashes: Mutex<VecDeque<Hash>>,
    pub sends: Mutex<u32>,
    /// `unknown` for every sign: no status is ever found and the blockhash is already past.
    pub never_lands: Mutex<bool>,
}

impl Node {
    fn blockhash(&self) -> Hash {
        let Ok(mut q) = self.blockhashes.lock() else {
            return Hash::default();
        };
        if q.len() > 1 {
            q.pop_front().unwrap_or_default()
        } else {
            q.front().cloned().unwrap_or_default()
        }
    }

    pub fn sends(&self) -> u32 {
        self.sends.lock().map_or(0, |s| *s)
    }

    fn lost(&self) -> bool {
        self.never_lands.lock().is_ok_and(|l| *l)
    }

    fn answer(&self, method: &str, params: &Value) -> Value {
        let lost = self.lost();
        match method {
            "getGenesisHash" => json!(ZINK_TESTNET.genesis_hash),
            "getLatestBlockhash" => {
                json!({"value": {"blockhash": self.blockhash().to_string(), "lastValidBlockHeight": 1_000}})
            }
            "getBlockHeight" => json!(if lost { 2_000 } else { 10 }),
            "getBalance" => json!({"value": BALANCE}),
            "getMultipleAccounts" => {
                let n = params[0].as_array().map_or(0, Vec::len);
                json!({"value": vec![Value::Null; n]})
            }
            "simulateTransaction" => {
                json!({"value": {"err": null, "logs": [], "unitsConsumed": 100,
                "accounts": [{"lamports": AFTER}]}})
            }
            "getFeeForMessage" => json!({"value": 0}),
            "sendTransaction" => {
                if let Ok(mut s) = self.sends.lock() {
                    *s = s.saturating_add(1);
                }
                json!("sent")
            }
            "getSignatureStatuses" if lost => json!({"value": [null]}),
            "getSignatureStatuses" => {
                json!({"value": [{"slot": 7, "err": null, "confirmationStatus": "confirmed"}]})
            }
            "getTransaction" => json!({"meta": {"fee": 0, "preBalances": [BALANCE],
                "postBalances": [AFTER], "logMessages": []}}),
            _ => Value::Null,
        }
    }
}

/// Starts a node on a random port; returns its URL.
pub fn start(node: &Arc<Node>) -> String {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let node = Arc::clone(node);
    std::thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let mut body = String::new();
            let _ = request.as_reader().read_to_string(&mut body);
            let msg: Value = serde_json::from_str(&body).unwrap_or_default();
            let method = msg["method"].as_str().unwrap_or_default();
            let result = node.answer(method, &msg["params"]);
            let reply = json!({"jsonrpc": "2.0", "id": msg["id"], "result": result});
            let _ = request.respond(tiny_http::Response::from_string(reply.to_string()));
        }
    });
    format!("http://127.0.0.1:{port}")
}

/// A temporary signer home: a config naming the node, and a session key `k` that may transfer to
/// [`dest`]. `extra` is appended to the config.
pub struct Home {
    pub dir: PathBuf,
    pub cfg: Config,
    pub pubkey: Address,
    pub node: Arc<Node>,
}

pub fn dest() -> Address {
    Address::new_from_array([2; 32])
}

impl Home {
    pub fn new(tag: &str, extra: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("sa-forge-signer-home-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pubkey = KeyStore::new(&dir.join("keys")).create("k").unwrap();
        let node = Arc::new(Node::default());
        node.blockhashes
            .lock()
            .unwrap()
            .push_back(Hash::new_from_array([3; 32]));
        let url = start(&node);
        let path = dir.join("config.toml");
        std::fs::write(
            &path,
            format!(
                "rpc_url = \"{url}\"\n[keys.k]\nclass = \"session\"\npubkey = \"{pubkey}\"\ntransfer_to = [\"{}\"]\n{extra}",
                dest()
            ),
        )
        .unwrap();
        let cfg = Config::load(&path, true).unwrap();
        Self {
            dir,
            cfg,
            pubkey,
            node,
        }
    }

    /// A payload moving `lamports` from the key to [`dest`].
    pub fn payload(&self, lamports: u64) -> String {
        crate::transfer::fund_payload(&self.pubkey, &dest(), lamports, "test").to_string()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
