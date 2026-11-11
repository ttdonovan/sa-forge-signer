//! Halts: a stop on signing for one key or all keys, kept under `state_dir/halts/` until a person
//! lifts it with `resume` at a terminal. Nothing lifts a halt on its own, and `serve` can only set one.

use std::fs::{DirBuilder, OpenOptions};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::audit::{Audit, Entry};
use crate::checks::{Refusal, refuse};
use crate::text::{MAX_PRINTED, printable};

/// Audit `key` of a halt or resume that covers every key.
pub const ALL_KEYS: &str = "*";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Halt {
    pub ts: u64,
    pub reason: String,
    /// Who set it: `cli`, `token <name>`, or `halt_after_unknown`.
    pub by: String,
}

impl Halt {
    pub fn refusal(&self, key: &str) -> Refusal {
        refuse(
            "halt",
            format!(
                "key {key:?} is halted (since {}, by {}): {}; only `sa-forge-signer resume` at a terminal lifts it",
                self.ts, self.by, self.reason
            ),
        )
    }
}

/// One key's halt, or `None` for the halt on every key.
fn path(state_dir: &Path, key: Option<&str>) -> PathBuf {
    let dir = state_dir.join("halts");
    key.map_or_else(
        || dir.join("all.json"),
        |k| dir.join("keys").join(format!("{k}.json")),
    )
}

fn read(path: &Path) -> Result<Option<Halt>> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .with_context(|| format!("parsing {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

/// The halt in force for `key`: its own, else the one on every key. An unreadable record is an
/// error, so a sign fails rather than passing a halt it cannot read.
pub fn active(state_dir: &Path, key: &str) -> Result<Option<Halt>> {
    let own = read(&path(state_dir, Some(key)))?;
    if own.is_some() {
        return Ok(own);
    }
    read(&path(state_dir, None))
}

/// Writes `bytes` to `file` (0600, in 0700 directories) whole, then renames it into place, so a
/// reader never sees half a record.
pub fn write_private(file: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(dir) = file.parent() {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .with_context(|| format!("creating {}", dir.display()))?;
    }
    let tmp = file.with_extension("json.tmp");
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .with_context(|| format!("opening {}", tmp.display()))?;
    f.write_all(bytes)?;
    f.sync_all()?;
    std::fs::rename(&tmp, file).with_context(|| format!("writing {}", file.display()))?;
    Ok(())
}

/// Writes the halt (one key, or every key for `None`) and its `halt` audit entry.
pub fn set(state_dir: &Path, key: Option<&str>, halt: &Halt) -> Result<()> {
    write_private(
        &path(state_dir, key),
        serde_json::to_string(halt)?.as_bytes(),
    )?;
    Audit::new(state_dir).append(Entry {
        ts: halt.ts,
        key: key.unwrap_or(ALL_KEYS).to_owned(),
        command: "halt".to_owned(),
        outcome: "halted".to_owned(),
        detail: Some(format!("{} (by {})", halt.reason, halt.by)),
        ..Entry::default()
    })?;
    Ok(())
}

/// Removes the halt (one key's, or the one on every key) after a person types `yes` on `tty`.
/// With no terminal it refuses. Returns whether a halt was removed.
pub fn resume<T: Read + Write>(
    state_dir: &Path,
    key: Option<&str>,
    tty: Option<T>,
    now: u64,
) -> Result<bool> {
    let file = path(state_dir, key);
    let scope = key.map_or_else(|| "every key".to_owned(), |k| format!("key {k:?}"));
    let Some(halt) = read(&file)? else {
        return Ok(false);
    };
    let Some(mut tty) = tty else {
        anyhow::bail!("resume needs a terminal: run it at one, never through a pipe or an agent");
    };
    write!(
        tty,
        "sa-forge-signer: resume signing with {scope}?\n  halted since {} by {}\n  reason (caller text): {}\nType 'yes' to resume: ",
        halt.ts,
        printable(&halt.by, MAX_PRINTED),
        printable(&halt.reason, MAX_PRINTED)
    )?;
    tty.flush()?;
    let mut answer = String::new();
    BufReader::new(&mut tty).read_line(&mut answer)?;
    if answer.trim() != "yes" {
        anyhow::bail!("not resumed");
    }
    std::fs::remove_file(&file).with_context(|| format!("removing {}", file.display()))?;
    Audit::new(state_dir).append(Entry {
        ts: now,
        key: key.unwrap_or(ALL_KEYS).to_owned(),
        command: "resume".to_owned(),
        outcome: "resumed".to_owned(),
        detail: Some(format!("lifted the halt by {}: {}", halt.by, halt.reason)),
        ..Entry::default()
    })?;
    Ok(true)
}

/// The controlling terminal, never stdin, so an agent piping input cannot resume.
pub fn terminal() -> Option<std::fs::File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty")
        .ok()
}

/// This key's `unknown` outcomes since its last landed outcome (`confirmed` or `failed`) or the
/// last `resume` covering it. Signs that sent nothing neither count nor reset.
#[cfg(test)]
pub fn unknown_streak(entries: &[Entry], key: &str) -> u32 {
    let mut streak: u32 = 0;
    for e in entries.iter().rev() {
        if !count_back(&mut streak, e, key) {
            break;
        }
    }
    streak
}

/// One step of the streak, newest entry first; false once the streak is decided.
fn count_back(streak: &mut u32, e: &Entry, key: &str) -> bool {
    if e.command == "resume" && (e.key == key || e.key == ALL_KEYS) {
        return false;
    }
    if e.key != key || e.command != "sign" {
        return true;
    }
    match e.outcome.as_str() {
        "unknown" | "expired" => *streak = streak.saturating_add(1),
        "confirmed" | "failed" => return false,
        _ => {}
    }
    true
}

/// After a sign, under the key lock: halts the key once its `unknown` streak reaches `limit`
/// (0 never trips). Returns whether it halted the key.
pub fn trip_after_unknown(state_dir: &Path, key: &str, limit: u32, now: u64) -> Result<bool> {
    if limit == 0 || active(state_dir, key)?.is_some() {
        return Ok(false);
    }
    let mut streak: u32 = 0;
    Audit::new(state_dir).walk_back(|e| count_back(&mut streak, e, key))?;
    if streak < limit {
        return Ok(false);
    }
    set(
        state_dir,
        Some(key),
        &Halt {
            ts: now,
            reason: format!(
                "{streak} consecutive unknown outcomes; reconcile those signatures and check the RPC before resuming"
            ),
            by: "halt_after_unknown".to_owned(),
        },
    )?;
    Ok(true)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::io::Cursor;

    /// A terminal that answers with `input` and records the prompt.
    pub struct Term {
        input: Cursor<Vec<u8>>,
        pub output: Vec<u8>,
    }

    impl Read for Term {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buf)
        }
    }

    impl Write for Term {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.output.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    pub fn term(answer: &str) -> Term {
        Term {
            input: Cursor::new(answer.as_bytes().to_vec()),
            output: Vec::new(),
        }
    }

    fn dir(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("sa-forge-signer-halt-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn halt(reason: &str) -> Halt {
        Halt {
            ts: 10,
            reason: reason.into(),
            by: "cli".into(),
        }
    }

    #[test]
    fn a_key_halt_covers_that_key_and_an_all_keys_halt_covers_every_key() {
        let d = dir("scope");
        assert_eq!(active(&d, "a").unwrap(), None);
        set(&d, Some("a"), &halt("one")).unwrap();
        assert_eq!(active(&d, "a").unwrap(), Some(halt("one")));
        assert_eq!(active(&d, "b").unwrap(), None);
        set(&d, None, &halt("all")).unwrap();
        assert_eq!(active(&d, "b").unwrap(), Some(halt("all")));
        assert_eq!(
            active(&d, "a").unwrap(),
            Some(halt("one")),
            "a key's own halt is reported first"
        );
        let r = halt("one").refusal("a");
        assert_eq!(r.check, "halt");
        assert!(r.detail.contains("one") && r.detail.contains("resume"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_unreadable_halt_is_an_error_not_a_pass() {
        let d = dir("corrupt");
        set(&d, Some("a"), &halt("x")).unwrap();
        std::fs::write(path(&d, Some("a")), "{").unwrap();
        assert!(active(&d, "a").is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn resume_needs_a_terminal_and_a_yes() {
        let d = dir("resume");
        set(&d, Some("a"), &halt("x")).unwrap();
        assert!(resume(&d, Some("a"), None::<Term>, 20).is_err());
        assert!(resume(&d, Some("a"), Some(term("no\n")), 20).is_err());
        assert!(
            !resume(&d, None, Some(term("yes\n")), 20).unwrap(),
            "the all-keys resume does not lift a key's own halt"
        );
        assert!(active(&d, "a").unwrap().is_some());
        let mut t = Some(term("yes\n"));
        assert!(resume(&d, Some("a"), t.as_mut(), 20).unwrap());
        assert!(
            String::from_utf8_lossy(&t.unwrap().output).contains("resume signing with key \"a\"")
        );
        assert_eq!(active(&d, "a").unwrap(), None);
        assert!(
            !resume(&d, Some("a"), Some(term("yes\n")), 21).unwrap(),
            "nothing left"
        );
        // Halt and resume entries chain in the audit log.
        let audit = Audit::new(&d);
        assert_eq!(audit.verify().unwrap(), Ok(2));
        let log = audit.entries().unwrap();
        assert_eq!(
            log.iter()
                .map(|e| (e.command.as_str(), e.key.as_str()))
                .collect::<Vec<_>>(),
            [("halt", "a"), ("resume", "a")]
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_resume_prompt_escapes_the_halt_reason() {
        let d = dir("resume-escape");
        set(&d, Some("a"), &halt("x\u{1b}[2K\rType 'yes'")).unwrap();
        let mut t = Some(term("no\n"));
        assert!(resume(&d, Some("a"), t.as_mut(), 20).is_err());
        let out = String::from_utf8(t.unwrap().output).unwrap();
        assert!(!out.contains('\u{1b}') && !out.contains('\r'), "{out:?}");
        assert!(
            out.contains("reason (caller text): x\\u{1b}[2K\\rType"),
            "{out:?}"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn signed(key: &str, outcome: &str) -> Entry {
        Entry {
            key: key.into(),
            command: "sign".into(),
            outcome: outcome.into(),
            ..Entry::default()
        }
    }

    #[test]
    fn the_unknown_streak_resets_on_a_landed_outcome_or_a_resume() {
        let u = || signed("k", "unknown");
        assert_eq!(unknown_streak(&[u(), u(), u()], "k"), 3);
        assert_eq!(
            unknown_streak(&[u(), signed("k", "confirmed"), u()], "k"),
            1
        );
        assert_eq!(unknown_streak(&[u(), signed("k", "failed"), u()], "k"), 1);
        assert_eq!(
            unknown_streak(
                &[
                    u(),
                    signed("k", "refused"),
                    u(),
                    signed("k", "simulation_failed")
                ],
                "k"
            ),
            2,
            "nothing sent: neither counts nor resets"
        );
        assert_eq!(
            unknown_streak(&[u(), signed("other", "confirmed"), u()], "k"),
            2
        );
        let resumed = |key: &str| Entry {
            key: key.into(),
            command: "resume".into(),
            ..Entry::default()
        };
        assert_eq!(unknown_streak(&[u(), u(), resumed("k"), u()], "k"), 1);
        assert_eq!(unknown_streak(&[u(), u(), resumed(ALL_KEYS), u()], "k"), 1);
        assert_eq!(unknown_streak(&[u(), u(), resumed("other"), u()], "k"), 3);
    }

    #[test]
    fn the_third_consecutive_unknown_trips_the_halt() {
        let d = dir("trip");
        let audit = Audit::new(&d);
        for n in 1..=3 {
            audit.append(signed("k", "unknown")).unwrap();
            assert!(!trip_after_unknown(&d, "k", 0, 5).unwrap(), "0 never trips");
            assert_eq!(trip_after_unknown(&d, "k", 3, 5).unwrap(), n == 3);
        }
        let h = active(&d, "k").unwrap().unwrap();
        assert_eq!(h.by, "halt_after_unknown");
        assert!(h.reason.starts_with("3 consecutive unknown"));
        assert!(
            !trip_after_unknown(&d, "k", 3, 6).unwrap(),
            "already halted"
        );
        assert_eq!(audit.entries().unwrap().last().unwrap().command, "halt");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
