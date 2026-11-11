//! Sending a signed transaction and waiting for a proven outcome.

use std::time::Duration;

use anyhow::{Context, Result};
use solana_transaction::Transaction;

use super::{Outcome, Report, encode, program_error};
use crate::rpc::{Chain, RpcError, Status};

pub const POLL: Duration = Duration::from_secs(1);
/// The first status check after sending: about a slot, since most transactions land within one.
pub const FIRST_POLL: Duration = Duration::from_millis(400);
pub const MAX_RPC_FAILURES: u32 = 60;
/// Longest a sign waits to prove an outcome. Expiry is proven against the finalized tip, which
/// trails the confirmed one, so this covers a blockhash's ~150-block life plus finalization; a
/// stalled or lagging RPC that never lets either proof complete ends here as `unknown`.
pub const CONFIRM_DEADLINE: Duration = Duration::from_secs(150);
pub const META_ATTEMPTS: u32 = 10;

/// The wait before status check `n` (from 1): [`FIRST_POLL`], doubling, never more than `poll`.
pub fn poll_delay(n: u32, poll: Duration) -> Duration {
    let doublings = n.saturating_sub(1).min(16);
    FIRST_POLL
        .checked_mul(1_u32 << doublings)
        .map_or(poll, |d| d.min(poll))
}

pub fn send_and_confirm(
    chain: &impl Chain,
    tx: &Transaction,
    last_valid: u64,
    mut report: Report,
    explorer: &str,
    poll: Duration,
    deadline: Duration,
) -> Result<Report> {
    let _span = tracing::info_span!("confirm").entered();
    let started = std::time::Instant::now();
    let wire = encode(tx)?;
    let signature = tx
        .signatures
        .first()
        .map(ToString::to_string)
        .context("transaction has no signature")?;
    report.signature = Some(signature.clone());
    report.explorer = Some(format!("{explorer}/tx/{signature}"));
    if let Err(e) = chain.send(&wire) {
        report.detail = Some(format!("send: {e}"));
    }
    let mut failures: u32 = 0;
    let mut polls: u32 = 0;
    // A landed status (confirmed or finalized) ends the wait from either lookup. Nothing proves
    // the opposite: once the finalized height is past last_valid, waiting cannot change the
    // outcome, but an empty status may come from a node other than the one that reported the
    // height, or one with incomplete history. So that ends the wait as `unknown`, never `expired`.
    report.outcome = loop {
        polls = polls.saturating_add(1);
        std::thread::sleep(poll_delay(polls, poll));
        match chain.signature_status(&signature) {
            Ok(Some(st)) if st.landed() => break landed_outcome(&mut report, st),
            Ok(_) => {}
            Err(e) => failures = note_failure(failures, &mut report, &e),
        }
        match chain.finalized_height() {
            Ok(height) if height > last_valid => match chain.signature_status_history(&signature) {
                Ok(Some(st)) if st.landed() => break landed_outcome(&mut report, st),
                Ok(_) => {
                    report.detail = Some(format!(
                        "blockhash expired (finalized height {height} is past {last_valid}) and the RPC reports no landed status; that does not prove it never landed: look up the signature and check the on-chain state before retrying"
                    ));
                    break Outcome::Unknown;
                }
                Err(e) => failures = note_failure(failures, &mut report, &e),
            },
            Ok(_) => {}
            Err(e) => failures = note_failure(failures, &mut report, &e),
        }
        if failures >= MAX_RPC_FAILURES {
            report.detail = Some(format!(
                "{}; look up the signature and check the on-chain state before retrying",
                report.detail.as_deref().unwrap_or("RPC failures")
            ));
            break Outcome::Unknown;
        }
        if started.elapsed() >= deadline {
            report.detail = Some(format!(
                "no proven outcome within {}s; look up the signature and check the on-chain state before retrying",
                deadline.as_secs()
            ));
            break Outcome::Unknown;
        }
        if polls.is_multiple_of(3) {
            let _ = chain.send(&wire);
        }
    };
    report.balance_change = None;
    if matches!(report.outcome, Outcome::Confirmed | Outcome::Failed) {
        // This transaction's own effect; a balance diff would include parallel transactions.
        // Bounded by the same deadline; without metadata the intent's reservation is charged.
        for _ in 0..META_ATTEMPTS {
            if started.elapsed() >= deadline {
                break;
            }
            if let Ok(Some(meta)) = chain.transaction_meta(&signature) {
                report.balance_change = meta.payer_change;
                report.fee = meta.fee.or(report.fee);
                if report.outcome == Outcome::Failed {
                    report.program_error = program_error(&meta.logs);
                }
                report.logs = meta.logs;
                break;
            }
            std::thread::sleep(poll);
        }
    }
    Ok(report)
}

pub fn landed_outcome(report: &mut Report, st: Status) -> Outcome {
    report.slot = Some(st.slot);
    match st.err {
        Some(err) => {
            report.error = Some(err);
            Outcome::Failed
        }
        None => Outcome::Confirmed,
    }
}

pub fn note_failure(failures: u32, report: &mut Report, e: &RpcError) -> u32 {
    report.detail = Some(e.to_string());
    failures.saturating_add(1)
}
