//! The reads behind `policy::evaluate`: the halt and approval records, the audit log, the clock and
//! the RPC.
//! RPC facts are read one at a time, in check order, so a refusal decided by earlier checks is
//! made before the RPC is called for the next.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;

use crate::audit::Entry;
use crate::checks;
use crate::config::KeyConfig;
use crate::engine::SIGN_INTENT;
use crate::halt;
use crate::policy::{Facts, KeyPolicy, Request, Usage};
use crate::policy_approval;
use crate::rpc::Rpc;

pub const MINUTE: u64 = 60;
pub const DAY: u64 = 86_400;

/// The facts read without the RPC: the halt, the approved policy, and the key's usage at `now` from
/// `entries`, its recent audit entries.
pub fn local(state_dir: &Path, key: &KeyConfig, entries: &[Entry], now: u64) -> Result<Facts> {
    let approved = if key.require_policy_approval {
        policy_approval::approved(state_dir, &key.name)?.map(|a| a.policy)
    } else {
        None
    };
    Ok(Facts {
        halt: halt::active(state_dir, &key.name)?,
        approved_policy: approved,
        usage: usage(entries, key, now),
        ..Facts::default()
    })
}

/// Reads the next RPC fact not yet read, in check order. Returns false once all are read.
pub fn read_next(
    facts: &mut Facts,
    rpc: &Rpc,
    request: &Request<'_>,
    policy: &KeyPolicy<'_>,
) -> Result<bool> {
    if facts.genesis_hash.is_none() {
        facts.genesis_hash = Some(rpc.genesis_hash()?);
    } else if facts.partial_signers_exist.is_none() {
        facts.partial_signers_exist = Some(rpc.accounts_exist(request.partial_signers)?);
    } else if facts.ata_owner_programs.is_none() {
        // A malformed create is refused by the check itself; nothing to look up for it.
        let owners =
            checks::ata_owners(request.message, &policy.structural(request)).unwrap_or_default();
        let programs = rpc.account_owners(&owners)?;
        facts.ata_owner_programs = Some(owners.into_iter().zip(programs).collect());
    } else {
        return Ok(false);
    }
    Ok(true)
}

/// Rate and daily spend from the audit log.
///
/// Every attempt counts toward the rate: final `sign` entries, plus intents that never got one
/// (the process died mid-sign). Spend counts what each sent transaction cost when that is known
/// (confirmed or failed, with metadata). When it is not known (the outcome is unknown, the
/// metadata never arrived, or the process died), the intent's simulated spend is charged as a
/// reservation, and with no simulation figure the whole daily cap is: an unresolved transaction
/// may have landed, so it is never free.
pub fn usage(entries: &[Entry], key: &KeyConfig, now: u64) -> Usage {
    let mine: Vec<&Entry> = entries.iter().filter(|e| e.key == key.name).collect();
    let finals = || mine.iter().filter(|e| e.command == "sign");
    // Sets and maps, not scans: a scan per entry made this quadratic in the window.
    let finished: HashSet<&str> = finals().filter_map(|e| e.signature.as_deref()).collect();
    let resolved = |sig: &str| finished.contains(sig);
    let mut intents: HashMap<&str, Option<i64>> = HashMap::new();
    for e in mine.iter().filter(|e| e.command == SIGN_INTENT) {
        if let Some(sig) = e.signature.as_deref() {
            intents.entry(sig).or_insert(e.balance_change);
        }
    }
    let orphan_intents = || {
        mine.iter().filter(|e| {
            e.command == SIGN_INTENT && e.signature.as_deref().is_none_or(|s| !resolved(s))
        })
    };
    let signs_last_minute = finals()
        .chain(orphan_intents())
        .filter(|e| e.ts >= now.saturating_sub(MINUTE))
        .count();
    let cap = key.daily_lamport_cap;
    let reservation = |sig: Option<&str>| -> u64 {
        match sig.and_then(|s| intents.get(s)) {
            Some(Some(d)) => spend(*d),
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
            // `expired` appears only in older logs, from a proof that could be wrong.
            "unknown" | "expired" => reservation(e.signature.as_deref()),
            // Refused or failed simulation: never sent.
            _ => 0,
        };
        spent = spent.saturating_add(cost);
    }
    for e in orphan_intents().filter(day) {
        spent = spent.saturating_add(e.balance_change.map_or(cap, spend));
    }
    Usage {
        signs_last_minute,
        spent,
    }
}

/// Lamports spent by a balance change (gains spend nothing).
pub const fn spend(delta: i64) -> u64 {
    if delta < 0 { delta.unsigned_abs() } else { 0 }
}
