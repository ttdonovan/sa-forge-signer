//! The parts of the signer's config the console shows. Lenient: unknown fields are ignored.
//! Defaults mirror `src/config.rs`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

pub const DEFAULT_RATE_PER_MINUTE: u32 = 30;
pub const DEFAULT_DAILY_LAMPORT_CAP: u64 = 50_000_000;
/// `zink-testnet`'s explorer, from `src/cluster.rs`.
pub const DEFAULT_EXPLORER: &str = "https://explorer.z.ink";

#[derive(Debug, Default, Deserialize)]
struct Raw {
    cluster: Option<String>,
    explorer_url: Option<String>,
    state_dir: Option<PathBuf>,
    #[serde(default)]
    keys: BTreeMap<String, RawKey>,
    serve: Option<RawServe>,
}

#[derive(Debug, Deserialize)]
struct RawKey {
    class: Option<String>,
    approval: Option<String>,
    rate_limit_per_minute: Option<u32>,
    daily_lamport_cap: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct RawServe {
    listen: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Key {
    pub name: String,
    pub class: String,
    pub approval: String,
    pub rate_limit_per_minute: u32,
    pub daily_lamport_cap: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Config {
    pub state_dir: Option<PathBuf>,
    pub keys: Vec<Key>,
    /// `[serve] listen`, when the config has a `[serve]` table.
    pub listen: Option<String>,
    /// Block explorer base URL without a trailing slash; none for a cluster the console does not know.
    pub explorer_url: Option<String>,
}

/// `$SA_FORGE_SIGNER_CONFIG`, else `~/.config/sa-forge-signer/config.toml`, as the signer does.
pub fn default_path() -> Option<PathBuf> {
    std::env::var_os("SA_FORGE_SIGNER_CONFIG")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|h| PathBuf::from(h).join(".config/sa-forge-signer/config.toml"))
        })
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let raw: Raw =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let state_dir = raw.state_dir.unwrap_or_else(|| PathBuf::from("state"));
        let keys = raw
            .keys
            .into_iter()
            .map(|(name, k)| {
                let class = k.class.unwrap_or_else(|| "?".to_owned());
                let approval = k.approval.unwrap_or_else(|| {
                    if class == "wallet" { "confirm" } else { "auto" }.to_owned()
                });
                Key {
                    name,
                    class,
                    approval,
                    rate_limit_per_minute: k
                        .rate_limit_per_minute
                        .unwrap_or(DEFAULT_RATE_PER_MINUTE),
                    daily_lamport_cap: k.daily_lamport_cap.unwrap_or(DEFAULT_DAILY_LAMPORT_CAP),
                }
            })
            .collect();
        Ok(Self {
            state_dir: Some(if state_dir.is_absolute() {
                state_dir
            } else {
                base.join(state_dir)
            }),
            keys,
            explorer_url: raw
                .explorer_url
                .map(|u| u.trim_end_matches('/').to_owned())
                .or_else(|| {
                    matches!(raw.cluster.as_deref(), None | Some("zink-testnet"))
                        .then(|| DEFAULT_EXPLORER.to_owned())
                }),
            listen: raw
                .serve
                .map(|s| s.listen.unwrap_or_else(|| "127.0.0.1:8790".to_owned())),
        })
    }
}
