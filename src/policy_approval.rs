//! Policy approval (hardening proposal 7): the policy digest a person last approved for a key, kept
//! under `state_dir/policy/`. A key with `require_policy_approval` signs only while they match.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::audit::{Audit, Entry};
use crate::checks::{Refusal, refuse};
use crate::config::KeyConfig;
use crate::halt::write_private;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Approved {
    pub ts: u64,
    pub policy: String,
}

fn path(state_dir: &Path, key: &str) -> PathBuf {
    state_dir.join("policy").join(format!("{key}.json"))
}

/// The key's approved digest. An unreadable record is an error, so a sign fails rather than pass.
pub fn approved(state_dir: &Path, key: &str) -> Result<Option<Approved>> {
    let file = path(state_dir, key);
    match std::fs::read_to_string(&file) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .with_context(|| format!("parsing {}", file.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {}", file.display())),
    }
}

/// The `policy` refusal, for a key that requires approval and whose digest is not `approved`.
pub fn check(key: &KeyConfig, approved: Option<&str>) -> Result<(), Refusal> {
    if !key.require_policy_approval || approved == Some(key.policy.as_str()) {
        return Ok(());
    }
    Err(refuse(
        "policy",
        format!(
            "key {:?} requires policy approval, and its policy {} is not the approved one ({}); check it with `sa-forge-signer config show`, then run `sa-forge-signer key approve {}` at a terminal",
            key.name,
            key.policy,
            approved.map_or_else(|| "none approved".to_owned(), |a| format!("approved: {a}")),
            key.name
        ),
    ))
}

/// Records the key's current digest as approved after a person types `yes` on `tty`, with its
/// audit entry. With no terminal it refuses. Returns false if that digest was already approved.
pub fn approve<T: Read + Write>(
    state_dir: &Path,
    key: &KeyConfig,
    tty: Option<T>,
    now: u64,
) -> Result<bool> {
    let before = approved(state_dir, &key.name)?.map(|a| a.policy);
    if before.as_deref() == Some(key.policy.as_str()) {
        return Ok(false);
    }
    let Some(mut tty) = tty else {
        bail!("key approve needs a terminal: run it at one, never through a pipe or an agent");
    };
    let was = before.as_deref().unwrap_or("none");
    write!(
        tty,
        "sa-forge-signer: approve the policy of key {:?}?\n  policy:   {}\n  approved: {was}\nCheck it with `sa-forge-signer config show` first. Type 'yes' to approve: ",
        key.name, key.policy
    )?;
    tty.flush()?;
    let mut answer = String::new();
    BufReader::new(&mut tty).read_line(&mut answer)?;
    if answer.trim() != "yes" {
        bail!("not approved");
    }
    let record = Approved {
        ts: now,
        policy: key.policy.clone(),
    };
    write_private(
        &path(state_dir, &key.name),
        serde_json::to_string(&record)?.as_bytes(),
    )?;
    Audit::new(state_dir).append(Entry {
        ts: now,
        key: key.name.clone(),
        command: "approve".to_owned(),
        outcome: "approved".to_owned(),
        detail: Some(format!("replaces {was}")),
        policy: Some(key.policy.clone()),
        ..Entry::default()
    })?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::Audit;
    use crate::halt::tests::term;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "sa-forge-signer-policy-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn key(policy: &str, required: bool) -> KeyConfig {
        let text = format!(
            "[keys.k]\nclass = \"session\"\npubkey = \"7JJhMrnvgqxS6MW59QQcr2Ki1NjZ7SrAjST4YtAFE8zR\"\nrequire_policy_approval = {required}\n"
        );
        let mut cfg = crate::config::Config::load_str(&text).unwrap();
        let mut k = cfg.keys.remove(0);
        k.policy = policy.into();
        k
    }

    #[test]
    fn approve_needs_a_terminal_and_a_yes() {
        let d = dir("approve");
        let k = key("p1", true);
        assert!(approve(&d, &k, None::<crate::halt::tests::Term>, 5).is_err());
        assert!(approve(&d, &k, Some(term("no\n")), 5).is_err());
        assert_eq!(approved(&d, "k").unwrap(), None);
        let mut t = term("yes\n");
        assert!(approve(&d, &k, Some(&mut t), 5).unwrap());
        let prompt = String::from_utf8(t.output).unwrap();
        assert!(
            prompt.contains("p1") && prompt.contains("config show"),
            "{prompt}"
        );
        assert_eq!(
            approved(&d, "k").unwrap(),
            Some(Approved {
                ts: 5,
                policy: "p1".into()
            })
        );
        let log = Audit::new(&d).entries().unwrap();
        let last = log.last().unwrap();
        assert_eq!(
            (
                last.command.as_str(),
                last.key.as_str(),
                last.policy.as_deref()
            ),
            ("approve", "k", Some("p1"))
        );
        assert!(
            !approve(&d, &k, None::<crate::halt::tests::Term>, 6).unwrap(),
            "the same digest needs no second approval"
        );
        let changed = key("p2", true);
        assert!(approve(&d, &changed, None::<crate::halt::tests::Term>, 7).is_err());
        assert!(approve(&d, &changed, Some(term("yes\n")), 7).unwrap());
        assert_eq!(approved(&d, "k").unwrap().unwrap().policy, "p2");
        assert!(Audit::new(&d).verify().unwrap().is_ok());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn only_a_key_that_requires_approval_is_refused() {
        assert_eq!(check(&key("p1", false), None), Ok(()));
        assert_eq!(check(&key("p1", true), Some("p1")), Ok(()));
        for stale in [None, Some("p0")] {
            let r = check(&key("p1", true), stale).unwrap_err();
            assert_eq!(r.check, "policy");
            assert!(
                r.detail.contains("p1") && r.detail.contains("key approve k"),
                "{}",
                r.detail
            );
        }
    }

    #[test]
    fn an_unreadable_record_is_an_error_not_a_pass() {
        let d = dir("corrupt");
        std::fs::create_dir_all(d.join("policy")).unwrap();
        std::fs::write(path(&d, "k"), "{").unwrap();
        assert!(approved(&d, "k").is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
