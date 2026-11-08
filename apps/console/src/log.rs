//! Read-only view of the signer's audit log: parse, verify the hash chain, and per-key usage.
//! Mirrors `src/audit.rs` and `limits` in `src/engine.rs`; change them together.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
pub const SIGN_INTENT: &str = "sign-intent";
const MINUTE: u64 = 60;
const DAY: u64 = 86_400;

/// Field order matters: the hash is over this struct serialized with `hash` empty.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Entry {
    pub ts: u64,
    pub key: String,
    pub command: String,
    pub payload_hash: String,
    pub programs: Vec<String>,
    pub summary: String,
    pub intent: Option<String>,
    pub outcome: String,
    pub detail: Option<String>,
    pub signature: Option<String>,
    pub slot: Option<u64>,
    pub balance_change: Option<i64>,
    /// Left out when empty, as the signer writes it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
    /// The key's policy digest; left out when absent, as the signer writes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,
    pub prev: String,
    pub hash: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub check: String,
    pub detail: String,
}

impl Entry {
    fn digest(&self) -> Option<String> {
        let mut unsigned = self.clone();
        unsigned.hash = String::new();
        let bytes = serde_json::to_vec(&unsigned).ok()?;
        Some(
            Sha256::digest(bytes)
                .iter()
                .fold(String::new(), |mut s, b| {
                    let _ = write!(s, "{b:02x}");
                    s
                }),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chain {
    Ok,
    /// 1-based line number and why.
    Broken(usize, String),
}

#[derive(Debug, Clone)]
pub struct Snapshot {
    /// Entries up to the first unreadable line.
    pub entries: Vec<Entry>,
    pub chain: Chain,
    /// Signatures with a final `sign` entry.
    resolved: HashSet<String>,
}

impl Snapshot {
    pub fn empty() -> Self {
        Self::parse("")
    }

    /// Parses complete lines only: a trailing line without a newline may still be being written.
    pub fn parse(text: &str) -> Self {
        let complete = text.rfind('\n').and_then(|i| text.get(..i)).unwrap_or("");
        let mut entries = Vec::new();
        let mut chain = Chain::Ok;
        let mut prev = GENESIS.to_owned();
        for (i, line) in complete.lines().enumerate() {
            let n = i.saturating_add(1);
            let entry: Entry = match serde_json::from_str(line) {
                Ok(e) => e,
                Err(e) => {
                    chain = Chain::Broken(n, format!("unreadable: {e}"));
                    break;
                }
            };
            if chain == Chain::Ok {
                if entry.prev != prev {
                    chain = Chain::Broken(n, "prev does not match the previous entry".to_owned());
                } else if entry.digest().as_deref() != Some(entry.hash.as_str()) {
                    chain = Chain::Broken(n, "hash does not match its content".to_owned());
                }
            }
            prev.clone_from(&entry.hash);
            entries.push(entry);
        }
        let resolved = entries
            .iter()
            .filter(|e| e.command == "sign")
            .filter_map(|e| e.signature.clone())
            .collect();
        Self {
            entries,
            chain,
            resolved,
        }
    }

    pub fn read(path: &Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::empty()),
            Err(e) => Err(e),
        }
    }

    /// A sent transaction whose outcome is not known: needs reconciling before a retry.
    pub fn unresolved(&self, e: &Entry) -> bool {
        match e.command.as_str() {
            SIGN_INTENT => e
                .signature
                .as_ref()
                .is_none_or(|s| !self.resolved.contains(s)),
            "sign" => matches!(e.outcome.as_str(), "unknown" | "expired"),
            _ => false,
        }
    }

    /// One row per signature: a `sign` entry merged with the `sign-intent` before it (same key and
    /// signature). An intent with no `sign` stays a row of its own; so does every other entry.
    /// Oldest first.
    pub fn rows(&self) -> Vec<Row> {
        let intents: HashMap<(&str, &str), usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.command == SIGN_INTENT)
            .filter_map(|(i, e)| Some(((e.key.as_str(), e.signature.as_deref()?), i)))
            .collect();
        let mut merged = HashSet::new();
        let mut rows: Vec<Row> = self
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let intent = (e.command == "sign")
                    .then(|| intents.get(&(e.key.as_str(), e.signature.as_deref()?)))
                    .flatten()
                    .copied()
                    .filter(|&j| j < i);
                if let Some(j) = intent {
                    merged.insert(j);
                }
                Row { entry: i, intent }
            })
            .collect();
        rows.retain(|r| !merged.contains(&r.entry));
        rows
    }

    /// Key names in order of first appearance.
    pub fn keys(&self) -> Vec<String> {
        let mut seen = Vec::<String>::new();
        for e in &self.entries {
            if !seen.contains(&e.key) {
                seen.push(e.key.clone());
            }
        }
        seen
    }

    /// The signer's view of a key's limits at `now`, as `limits` in `src/engine.rs` computes it.
    pub fn usage(&self, key: &str, cap: u64, now: u64) -> Usage {
        let mine: Vec<&Entry> = self.entries.iter().filter(|e| e.key == key).collect();
        let finals = || mine.iter().filter(|e| e.command == "sign");
        let orphans = || {
            mine.iter().filter(|e| {
                e.command == SIGN_INTENT
                    && e.signature
                        .as_ref()
                        .is_none_or(|s| !self.resolved.contains(s))
            })
        };
        let recent = finals()
            .chain(orphans())
            .filter(|e| e.ts >= now.saturating_sub(MINUTE))
            .count();
        let reservation = |sig: Option<&str>| -> u64 {
            let intent = mine.iter().find(|e| {
                e.command == SIGN_INTENT && sig.is_some() && e.signature.as_deref() == sig
            });
            match intent.map(|e| e.balance_change) {
                Some(Some(d)) => spend(d),
                _ => cap,
            }
        };
        let day = |e: &&&Entry| e.ts >= now.saturating_sub(DAY);
        let mut spent: u64 = 0;
        for e in finals().filter(day) {
            let cost = match e.outcome.as_str() {
                "confirmed" | "failed" => e
                    .balance_change
                    .map_or_else(|| reservation(e.signature.as_deref()), spend),
                "unknown" | "expired" => reservation(e.signature.as_deref()),
                _ => 0,
            };
            spent = spent.saturating_add(cost);
        }
        for e in orphans().filter(day) {
            spent = spent.saturating_add(e.balance_change.map_or(cap, spend));
        }
        let unresolved = mine.iter().filter(|e| self.unresolved(e)).count();
        Usage {
            recent,
            spent,
            unresolved,
        }
    }
}

/// A row of the log view: an entry, and the `sign-intent` merged into it, as indexes into `entries`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub entry: usize,
    pub intent: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    /// Signs in the last minute, as counted against the rate limit.
    pub recent: usize,
    /// Lamports spent or reserved in the last 24 h.
    pub spent: u64,
    /// Unresolved sends, of any age.
    pub unresolved: usize,
}

const fn spend(delta: i64) -> u64 {
    if delta < 0 { delta.unsigned_abs() } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a valid chain the way `Audit::append` does.
    fn chain(entries: Vec<Entry>) -> String {
        let mut prev = GENESIS.to_owned();
        let mut out = String::new();
        for mut e in entries {
            e.prev.clone_from(&prev);
            e.hash = e.digest().unwrap();
            prev.clone_from(&e.hash);
            out.push_str(&serde_json::to_string(&e).unwrap());
            out.push('\n');
        }
        out
    }

    fn entry(ts: u64, command: &str, outcome: &str, sig: &str, change: Option<i64>) -> Entry {
        Entry {
            ts,
            key: "k".into(),
            command: command.into(),
            outcome: outcome.into(),
            signature: Some(sig.into()),
            balance_change: change,
            ..Entry::default()
        }
    }

    #[test]
    fn verifies_and_detects_tampering() {
        let text = chain(vec![
            entry(1, SIGN_INTENT, "pending", "a", Some(-5)),
            entry(2, "sign", "confirmed", "a", Some(-5)),
        ]);
        assert_eq!(Snapshot::parse(&text).chain, Chain::Ok);
        let tampered = text.replacen("\"ts\":2", "\"ts\":3", 1);
        assert!(matches!(
            Snapshot::parse(&tampered).chain,
            Chain::Broken(2, _)
        ));
    }

    /// Written by `sa-forge-signer sign`: catches drift from `src/audit.rs`'s `Entry`.
    #[test]
    fn verifies_a_log_the_signer_wrote() {
        let s = Snapshot::parse(include_str!("../tests/fixtures/signer-audit.jsonl"));
        assert_eq!((s.entries.len(), s.chain), (2, Chain::Ok));
    }

    /// Also written by the signer: an entry with instruction-table findings.
    #[test]
    fn verifies_an_entry_with_findings() {
        let s = Snapshot::parse(include_str!(
            "../tests/fixtures/signer-audit-findings.jsonl"
        ));
        assert_eq!((s.entries.len(), s.chain), (1, Chain::Ok));
    }

    /// Also written by the signer: a `key approve` entry and a sign, each with a policy digest.
    #[test]
    fn verifies_entries_with_a_policy() {
        let s = Snapshot::parse(include_str!("../tests/fixtures/signer-audit-policy.jsonl"));
        assert_eq!((s.entries.len(), s.chain), (2, Chain::Ok));
    }

    #[test]
    fn ignores_a_line_still_being_written() {
        let mut text = chain(vec![entry(1, "sign", "confirmed", "a", None)]);
        text.push_str("{\"ts\":");
        let s = Snapshot::parse(&text);
        assert_eq!((s.entries.len(), s.chain), (1, Chain::Ok));
    }

    #[test]
    fn merges_each_sign_with_its_intent() {
        let mut other_key = entry(5, "sign", "confirmed", "c", None);
        other_key.key = "other".into();
        let s = Snapshot::parse(&chain(vec![
            entry(1, SIGN_INTENT, "pending", "a", Some(-10)),
            entry(2, SIGN_INTENT, "pending", "b", Some(-10)),
            entry(3, "sign", "confirmed", "a", Some(-5)),
            // Orphan: no final entry, stays its own row.
            entry(4, SIGN_INTENT, "pending", "c", Some(-10)),
            // Same signature under another key: not merged.
            other_key,
            entry(6, "sign", "unknown", "b", None),
        ]));
        let rows: Vec<(usize, Option<usize>)> =
            s.rows().iter().map(|r| (r.entry, r.intent)).collect();
        assert_eq!(rows, [(2, Some(0)), (3, None), (4, None), (5, Some(1))]);
        // A refused sign never wrote an intent.
        let refused = Snapshot::parse(include_str!("../tests/fixtures/signer-audit.jsonl"));
        assert_eq!(refused.rows().len(), 2);
    }

    #[test]
    fn usage_matches_the_signers_limits() {
        let now = 100_000;
        let s = Snapshot::parse(&chain(vec![
            // Confirmed: charged its own spend.
            entry(now - 10, SIGN_INTENT, "pending", "a", Some(-100)),
            entry(now - 9, "sign", "confirmed", "a", Some(-90)),
            // Unknown: charged the intent's reservation.
            entry(now - 8, SIGN_INTENT, "pending", "b", Some(-600)),
            entry(now - 7, "sign", "unknown", "b", None),
            // Orphan with no figure: charged the whole cap.
            entry(now - 6, SIGN_INTENT, "pending", "c", None),
            // Older than a day: not charged, but still unresolved.
            entry(now - DAY - 1, SIGN_INTENT, "pending", "d", Some(-1)),
        ]));
        let u = s.usage("k", 1_000, now);
        assert_eq!(u.spent, 90 + 600 + 1_000);
        assert_eq!(u.recent, 3);
        assert_eq!(u.unresolved, 3);
    }
}
