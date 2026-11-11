//! Append-only, hash-chained JSONL audit log, and the per-key signing lock.
//!
//! The signer writes no key material here. `summary` and `intent` are caller-supplied text and
//! are recorded as given, up to [`crate::text::MAX_TEXT`] characters: callers must not put secrets in them.

use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::payload::hex;

/// How far back the checks read: the 24 h window, plus a day of slack for a clock stepped back.
pub const LOOKBACK: u64 = 2 * 86_400;
const CHUNK: u64 = 64 * 1024;

const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Entry {
    pub ts: u64,
    pub key: String,
    pub command: String,
    pub payload_hash: String,
    pub programs: Vec<String>,
    /// Builder-supplied text; not verified.
    pub summary: String,
    pub intent: Option<String>,
    pub outcome: String,
    pub detail: Option<String>,
    pub signature: Option<String>,
    pub slot: Option<u64>,
    pub balance_change: Option<i64>,
    /// The instruction table's findings. Left out when empty, so entries written before it keep
    /// their hashes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
    /// The key's policy digest (hardening proposal 7). Left out when absent, as `findings` is.
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
    fn digest(&self) -> Result<String> {
        let mut unsigned = self.clone();
        unsigned.hash = String::new();
        Ok(hex(&Sha256::digest(serde_json::to_vec(&unsigned)?)))
    }
}

/// An exclusive per-key lock under `state_dir/locks/`, held for a whole sign: from the limit
/// check, through the pre-send intent entry and confirmation, to the final audit entry. It is a
/// file lock, so it serializes signs across `serve` threads and separate CLI processes alike.
pub struct KeyLock {
    file: File,
}

impl KeyLock {
    pub fn acquire(state_dir: &Path, key: &str) -> Result<Self> {
        let dir = state_dir.join("locks");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join(format!("{key}.lock"));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        file.lock()
            .with_context(|| format!("locking {}", path.display()))?;
        Ok(Self { file })
    }
}

impl Drop for KeyLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

/// The lines of a file from the last back, without their newlines; blank lines are skipped.
struct RevLines<'a> {
    file: &'a File,
    chunk: u64,
    /// Where `buf` starts in the file; everything before it is unread.
    pos: u64,
    /// Read but not yet returned: the end of a line whose start may still be unread.
    buf: Vec<u8>,
}

impl<'a> RevLines<'a> {
    fn new(file: &'a File, chunk: u64) -> Result<Self> {
        Ok(Self {
            file,
            chunk: chunk.max(1),
            pos: file.metadata()?.len(),
            buf: Vec::new(),
        })
    }

    fn next_line(&mut self) -> Result<Option<Vec<u8>>> {
        loop {
            if let Some(i) = self.buf.iter().rposition(|b| *b == b'\n') {
                let line = self.buf.split_off(i.saturating_add(1));
                self.buf.truncate(i);
                if !line.is_empty() {
                    return Ok(Some(line));
                }
                continue;
            }
            if self.pos == 0 {
                let line = std::mem::take(&mut self.buf);
                return Ok((!line.is_empty()).then_some(line));
            }
            let n = self.chunk.min(self.pos);
            self.pos = self.pos.saturating_sub(n);
            let mut head = vec![0; usize::try_from(n)?];
            let mut file = self.file;
            file.seek(SeekFrom::Start(self.pos))?;
            file.read_exact(&mut head)?;
            head.append(&mut self.buf);
            self.buf = head;
        }
    }
}

pub struct Audit {
    path: PathBuf,
}

impl Audit {
    pub fn new(state_dir: &Path) -> Self {
        Self {
            path: state_dir.join("audit.jsonl"),
        }
    }

    fn entries_from(file: &File) -> Result<Vec<Entry>> {
        BufReader::new(file)
            .lines()
            .enumerate()
            .map(|(i, line)| {
                let line = line?;
                serde_json::from_str(&line)
                    .with_context(|| format!("audit line {}", i.saturating_add(1)))
            })
            .collect()
    }

    /// Calls `more` on each entry from the newest back, until it returns false or the log ends.
    /// Reads only as far back as that, so a sign's cost does not grow with the log's age.
    pub fn walk_back(&self, mut more: impl FnMut(&Entry) -> bool) -> Result<()> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e).with_context(|| format!("reading {}", self.path.display())),
        };
        let mut lines = RevLines::new(&file, CHUNK)?;
        let mut back: usize = 0;
        while let Some(line) = lines.next_line()? {
            back = back.saturating_add(1);
            let entry: Entry = serde_json::from_slice(&line)
                .with_context(|| format!("audit line {back} from the end"))?;
            if !more(&entry) {
                break;
            }
        }
        Ok(())
    }

    /// The entries from the newest back to the first older than `now - LOOKBACK`, oldest first.
    pub fn recent(&self, now: u64) -> Result<Vec<Entry>> {
        let since = now.saturating_sub(LOOKBACK);
        let mut out = Vec::new();
        self.walk_back(|e| {
            let keep = e.ts >= since;
            if keep {
                out.push(e.clone());
            }
            keep
        })?;
        out.reverse();
        Ok(out)
    }

    pub fn entries(&self) -> Result<Vec<Entry>> {
        match File::open(&self.path) {
            Ok(f) => Self::entries_from(&f),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e).with_context(|| format!("reading {}", self.path.display())),
        }
    }

    /// Appends under an exclusive file lock, chaining to the last entry.
    pub fn append(&self, mut entry: Entry) -> Result<Entry> {
        let _span = tracing::info_span!("audit_append", command = %entry.command).entered();
        if let Some(dir) = self.path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .create(dir)
                .with_context(|| format!("creating {}", dir.display()))?;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .append(true)
            .create(true)
            .mode(0o600)
            .open(&self.path)
            .with_context(|| format!("opening {}", self.path.display()))?;
        file.lock()?;
        let last = match RevLines::new(&file, CHUNK)?.next_line()? {
            Some(line) => {
                Some(serde_json::from_slice::<Entry>(&line).context("the last audit line")?)
            }
            None => None,
        };
        entry.prev = last.map_or_else(|| GENESIS.to_owned(), |e| e.hash);
        entry.hash = entry.digest()?;
        let mut line = serde_json::to_string(&entry)?;
        line.push('\n');
        file.write_all(line.as_bytes())?;
        file.sync_all()?;
        file.unlock()?;
        Ok(entry)
    }

    /// Returns the number of entries, or the first line that breaks the chain.
    pub fn verify(&self) -> Result<std::result::Result<usize, String>> {
        let entries = self.entries()?;
        let mut prev = GENESIS.to_owned();
        for (i, e) in entries.iter().enumerate() {
            let line = i.saturating_add(1);
            if e.prev != prev {
                return Ok(Err(format!(
                    "line {line}: prev does not match the previous entry"
                )));
            }
            if e.digest()? != e.hash {
                return Ok(Err(format!("line {line}: hash does not match its content")));
            }
            prev.clone_from(&e.hash);
        }
        Ok(Ok(entries.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "sa-forge-signer-audit-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn at(ts: u64, summary: &str) -> Entry {
        Entry {
            ts,
            key: "k".into(),
            command: "sign".into(),
            summary: summary.into(),
            ..Entry::default()
        }
    }

    #[test]
    fn lines_are_read_back_across_chunk_boundaries() {
        let d = dir("chunks");
        std::fs::create_dir_all(&d).unwrap();
        let path = d.join("lines");
        let long = "x".repeat(20);
        std::fs::write(&path, format!("a\n{long}\n\nbc\n")).unwrap();
        for chunk in [1, 3, 7, 64] {
            let file = File::open(&path).unwrap();
            let mut lines = RevLines::new(&file, chunk).unwrap();
            let mut got = Vec::new();
            while let Some(l) = lines.next_line().unwrap() {
                got.push(String::from_utf8(l).unwrap());
            }
            assert_eq!(got, ["bc", long.as_str(), "a"], "chunk {chunk}");
        }
        std::fs::write(&path, "no newline at the end").unwrap();
        let file = File::open(&path).unwrap();
        let mut lines = RevLines::new(&file, 4).unwrap();
        assert_eq!(
            lines.next_line().unwrap().unwrap(),
            b"no newline at the end"
        );
        assert_eq!(lines.next_line().unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn recent_stops_at_the_first_entry_past_the_lookback() {
        let d = dir("recent");
        let audit = Audit::new(&d);
        let now = 10 * LOOKBACK;
        for (ts, s) in [
            (now - LOOKBACK - 5, "too old"),
            (now - 1, "clock ahead"),
            (now - LOOKBACK - 1, "past the lookback"),
            (now - LOOKBACK + 1, "inside"),
            (now, "newest"),
        ] {
            audit.append(at(ts, s)).unwrap();
        }
        let got: Vec<String> = audit
            .recent(now)
            .unwrap()
            .into_iter()
            .map(|e| e.summary)
            .collect();
        assert_eq!(
            got,
            ["inside", "newest"],
            "oldest first, and nothing behind the stop"
        );
        let mut seen = Vec::new();
        audit
            .walk_back(|e| {
                seen.push(e.summary.clone());
                e.summary != "clock ahead"
            })
            .unwrap();
        assert_eq!(
            seen,
            ["newest", "inside", "past the lookback", "clock ahead"]
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    // A damaged old line no longer stops every sign; `audit verify` still finds it.
    #[test]
    fn append_reads_only_the_last_entry() {
        let d = dir("tail");
        let audit = Audit::new(&d);
        audit.append(at(1, "first")).unwrap();
        let path = d.join("audit.jsonl");
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, format!("not json\n{text}")).unwrap();
        let second = audit.append(at(2, "second")).unwrap();
        let first: Entry = serde_json::from_str(text.trim_end()).unwrap();
        assert_eq!(second.prev, first.hash);
        assert!(audit.verify().is_err(), "verify still reads every line");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn chain_appends_and_detects_tampering() {
        let dir =
            std::env::temp_dir().join(format!("sa-forge-signer-audit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let audit = Audit::new(&dir);
        for n in 0..3 {
            audit
                .append(Entry {
                    ts: n,
                    key: "k".into(),
                    outcome: "confirmed".into(),
                    ..Entry::default()
                })
                .unwrap();
        }
        assert_eq!(audit.verify().unwrap(), Ok(3));
        let path = dir.join("audit.jsonl");
        let text = std::fs::read_to_string(&path)
            .unwrap()
            .replacen("\"ts\":1", "\"ts\":9", 1);
        std::fs::write(&path, text).unwrap();
        assert!(audit.verify().unwrap().is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
