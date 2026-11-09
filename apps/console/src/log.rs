//! Read-only view of the signer's audit log: parse, verify the hash chain, and per-key usage.
//! Mirrors `src/audit.rs` and `limits` in `src/engine.rs`; change them together.

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
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
    /// The last entry's hash: where the chain continues.
    prev: String,
    /// Complete lines read so far, and the last of them with its newline.
    lines: usize,
    offset: u64,
    tail: Vec<u8>,
    /// An unreadable line was met: like `parse`, read no entries after it.
    stopped: bool,
}

impl Snapshot {
    pub fn empty() -> Self {
        Self::parse("")
    }

    /// Parses complete lines only: a trailing line without a newline may still be being written.
    pub fn parse(text: &str) -> Self {
        let mut s = Self {
            entries: Vec::new(),
            chain: Chain::Ok,
            resolved: HashSet::new(),
            prev: GENESIS.to_owned(),
            lines: 0,
            offset: 0,
            tail: Vec::new(),
            stopped: false,
        };
        s.extend(text);
        s
    }

    /// Adds the complete lines of `text`, the bytes that follow the last line read.
    fn extend(&mut self, text: &str) {
        let Some(complete) = text.rfind('\n').and_then(|i| text.get(..=i)) else {
            return;
        };
        for line in complete.lines() {
            self.lines = self.lines.saturating_add(1);
            if self.stopped {
                continue;
            }
            let entry: Entry = match serde_json::from_str(line) {
                Ok(e) => e,
                Err(e) => {
                    self.chain = Chain::Broken(self.lines, format!("unreadable: {e}"));
                    self.stopped = true;
                    continue;
                }
            };
            if self.chain == Chain::Ok {
                if entry.prev != self.prev {
                    self.chain = Chain::Broken(
                        self.lines,
                        "prev does not match the previous entry".to_owned(),
                    );
                } else if entry.digest().as_deref() != Some(entry.hash.as_str()) {
                    self.chain =
                        Chain::Broken(self.lines, "hash does not match its content".to_owned());
                }
            }
            self.prev.clone_from(&entry.hash);
            if entry.command == "sign"
                && let Some(sig) = &entry.signature
            {
                self.resolved.insert(sig.clone());
            }
            self.entries.push(entry);
        }
        let without_last = complete.len().saturating_sub(1);
        let start = complete
            .get(..without_last)
            .and_then(|c| c.rfind('\n'))
            .map_or(0, |i| i.saturating_add(1));
        self.tail = complete
            .as_bytes()
            .get(start..)
            .unwrap_or_default()
            .to_vec();
        self.offset = self
            .offset
            .saturating_add(u64::try_from(complete.len()).unwrap_or(u64::MAX));
    }

    pub fn read(path: &Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(Self::parse(&text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::empty()),
            Err(e) => Err(e),
        }
    }

    /// Reads what was appended since the last read; a log that shrank or changed before that point
    /// is read again whole. Equal to [`Snapshot::read`] for a log that is only appended to, but an
    /// edit above the last line read is not seen: read it whole now and then.
    pub fn refresh(&mut self, path: &Path) -> std::io::Result<()> {
        let mut file = match File::open(path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                *self = Self::empty();
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        if file.metadata()?.len() < self.offset || !self.tail_unchanged(&mut file)? {
            *self = Self::read(path)?;
            return Ok(());
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut added = Vec::new();
        file.read_to_end(&mut added)?;
        // Only complete lines: a character still being written must not fail the read.
        let end = added
            .iter()
            .rposition(|b| *b == b'\n')
            .map_or(0, |i| i.saturating_add(1));
        let text = std::str::from_utf8(added.get(..end).unwrap_or_default())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        self.extend(text);
        Ok(())
    }

    /// Whether the last line read is still where it was.
    fn tail_unchanged(&self, file: &mut File) -> std::io::Result<bool> {
        let len = u64::try_from(self.tail.len()).unwrap_or(u64::MAX);
        let Some(start) = self.offset.checked_sub(len) else {
            return Ok(false);
        };
        file.seek(SeekFrom::Start(start))?;
        let mut now = vec![0; self.tail.len()];
        file.read_exact(&mut now)?;
        Ok(now == self.tail)
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

    fn temp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!(
            "sa-forge-signer-console-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("audit.jsonl")
    }

    fn same(a: &Snapshot, b: &Snapshot) -> bool {
        let hashes = |s: &Snapshot| s.entries.iter().map(|e| e.hash.clone()).collect::<Vec<_>>();
        hashes(a) == hashes(b) && a.chain == b.chain && a.resolved == b.resolved
    }

    fn log_of(n: u64) -> String {
        chain(
            (0..n)
                .map(|i| {
                    entry(
                        i,
                        if i % 2 == 0 { SIGN_INTENT } else { "sign" },
                        "confirmed",
                        &format!("s{}", i / 2),
                        Some(-5),
                    )
                })
                .collect(),
        )
    }

    #[test]
    fn refresh_matches_a_full_read_as_the_log_grows() {
        let path = temp("grow");
        let text = format!("{}not json\n{}", log_of(4), log_of(1));
        let mut s = Snapshot::empty();
        // Every prefix, so lines arrive whole, in pieces, and several at once.
        for end in (0..=text.len()).step_by(37).chain([text.len()]) {
            std::fs::write(&path, text.as_bytes().get(..end).unwrap()).unwrap();
            s.refresh(&path).unwrap();
            assert!(
                same(&s, &Snapshot::read(&path).unwrap()),
                "after {end} bytes"
            );
        }
        assert!(matches!(s.chain, Chain::Broken(5, _)), "{:?}", s.chain);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn refresh_reads_a_rewritten_or_shrunk_log_again_whole() {
        let path = temp("rewrite");
        let mut s = Snapshot::empty();
        std::fs::write(&path, log_of(3)).unwrap();
        s.refresh(&path).unwrap();
        let tampered = log_of(4).replacen("\"ts\":2", "\"ts\":9", 1);
        std::fs::write(&path, &tampered).unwrap();
        s.refresh(&path).unwrap();
        assert!(
            matches!(s.chain, Chain::Broken(3, _)),
            "the line before the old end changed: {:?}",
            s.chain
        );
        std::fs::write(&path, log_of(1)).unwrap();
        s.refresh(&path).unwrap();
        assert!(same(&s, &Snapshot::read(&path).unwrap()), "shrunk");
        std::fs::remove_file(&path).unwrap();
        s.refresh(&path).unwrap();
        assert!(s.entries.is_empty(), "removed");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    // An edit above the last line read is not seen until a full read, which the app does each minute.
    #[test]
    fn refresh_parses_only_the_appended_lines() {
        let path = temp("append");
        let text = log_of(3);
        std::fs::write(&path, &text).unwrap();
        let mut s = Snapshot::empty();
        s.refresh(&path).unwrap();
        let edited = text.replacen("\"ts\":0", "\"ts\":7", 1);
        let longer = log_of(4);
        std::fs::write(
            &path,
            format!("{edited}{}", longer.get(text.len()..).unwrap()),
        )
        .unwrap();
        s.refresh(&path).unwrap();
        assert_eq!((s.entries.len(), s.chain.clone()), (4, Chain::Ok));
        assert!(matches!(
            Snapshot::read(&path).unwrap().chain,
            Chain::Broken(1, _)
        ));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
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
