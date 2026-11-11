//! The gates between simulation and sending that may rebuild the transaction with a fresh
//! blockhash: the person's approval, a stale blockhash and a repeated signature.

use anyhow::Result;
use solana_hash::Hash;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_transaction::Transaction;

use super::approval::confirm;
use super::{Outcome, Report, b64, build, encode_unsigned, program_error};
use crate::audit::Entry;
use crate::checks::refuse as refusal;
use crate::config::{Approval, KeyConfig};
use crate::facts::DAY;
use crate::payload::Payload;
use crate::rpc::{Rpc, RpcError, SimBlockhash, Simulation, delta};

/// The two calls a fresh rebuild needs; a trait so the rebuild can be tested without a node.
pub trait Fresh {
    fn balance(&self, address: &solana_address::Address) -> std::result::Result<u64, RpcError>;
    fn fee_for_message(&self, message_b64: &str) -> std::result::Result<Option<u64>, RpcError>;
    fn latest_blockhash(&self) -> std::result::Result<(Hash, u64), RpcError>;
    /// Simulates an unsigned copy (`tx_b64`) against its own blockhash.
    fn simulate_unsigned(
        &self,
        tx_b64: &str,
        accounts: &[solana_address::Address],
    ) -> std::result::Result<Simulation, RpcError>;
}

impl Fresh for Rpc {
    fn balance(&self, address: &solana_address::Address) -> std::result::Result<u64, RpcError> {
        Self::balance(self, address)
    }
    fn fee_for_message(&self, message_b64: &str) -> std::result::Result<Option<u64>, RpcError> {
        Self::fee_for_message(self, message_b64)
    }
    fn latest_blockhash(&self) -> std::result::Result<(Hash, u64), RpcError> {
        Self::latest_blockhash(self)
    }
    fn simulate_unsigned(
        &self,
        tx_b64: &str,
        accounts: &[solana_address::Address],
    ) -> std::result::Result<Simulation, RpcError> {
        self.simulate(tx_b64, SimBlockhash::Keep, accounts)
    }
}

/// Approval can outlast a blockhash, so the approved instructions are re-signed with a fresh one
/// and that exact transaction is simulated before it may be sent: it is new bytes. Its own
/// simulated spend and fee replace the earlier ones, so the cap check and the reservation see
/// what is sent. A failure comes back as the report to return (`SimulationFailed`, nothing sent).
pub fn after_approval(
    chain: &impl Fresh,
    payload: &Payload,
    signers: &[&Keypair],
    payer: &solana_address::Address,
    mut report: Report,
) -> Result<std::result::Result<(Transaction, u64, Report), Report>> {
    let pre = chain.balance(payer)?;
    let (tx, last_valid, sim) = fresh_simulated(chain, payload, signers, payer)?;
    report.compute_units = sim.units;
    report.balance_change = sim
        .post_lamports
        .first()
        .copied()
        .flatten()
        .and_then(|post| delta(pre, post));
    report.fee = chain
        .fee_for_message(&b64(&tx.message.serialize()))
        .ok()
        .flatten();
    if let Some(err) = sim.err {
        report.outcome = Outcome::SimulationFailed;
        report.program_error = program_error(&sim.logs);
        report.detail = Some(format!(
            "after approval: {}",
            report
                .program_error
                .clone()
                .unwrap_or_else(|| err.to_string())
        ));
        report.error = Some(err);
        report.logs = sim.logs;
        return Ok(Err(report));
    }
    Ok(Ok((tx, last_valid, report)))
}

/// The person's approval (a `confirm` key, then re-signed with a fresh blockhash), and a
/// blockhash still valid (`unless_stale`), with the checks passed recorded. A refusal or failure
/// comes back as the report to return, nothing sent.
pub fn approved_fresh(
    rpc: &Rpc,
    key: &KeyConfig,
    payload: &Payload,
    signers: &[&Keypair],
    tx: Transaction,
    last_valid: u64,
    mut r: Report,
) -> Result<std::result::Result<(Transaction, u64, Report), Report>> {
    let (tx, last_valid) = if key.approval == Approval::Confirm {
        let approved = {
            let _wait = tracing::info_span!("approval").entered();
            confirm(&r, key)?
        };
        if !approved {
            return Ok(Err(
                r.refused(&refusal("approval", "not approved".to_owned()))
            ));
        }
        match after_approval(rpc, payload, signers, &key.pubkey, r)? {
            Ok((fresh, last_valid, next)) => {
                r = next;
                (fresh, last_valid)
            }
            Err(failed) => return Ok(Err(failed)),
        }
    } else {
        (tx, last_valid)
    };
    r.checks.push("approval");
    let height = || rpc.finalized_height().ok();
    match unless_stale(
        rpc,
        height,
        tx,
        last_valid,
        payload,
        signers,
        &key.pubkey,
        r,
    )? {
        Ok((tx, last_valid, mut next)) => {
            next.checks.push("blockhash");
            Ok(Ok((tx, last_valid, next)))
        }
        Err(failed) => Ok(Err(failed)),
    }
}

/// A blockhash whose last valid height is already behind a finalized height can never land,
/// whichever node reports that height (a lagging node's blockhash, 2026-10-14 and 10-15). It is
/// re-signed and simulated once with a fresh one; still stale, it is refused, and nothing is sent,
/// so a retry is safe. A height that cannot be read changes nothing.
#[expect(
    clippy::too_many_arguments,
    reason = "the signed transaction, its signing inputs and the report; a struct would serve only the lint"
)]
pub fn unless_stale(
    chain: &impl Fresh,
    height: impl Fn() -> Option<u64>,
    tx: Transaction,
    last_valid: u64,
    payload: &Payload,
    signers: &[&Keypair],
    payer: &solana_address::Address,
    report: Report,
) -> Result<std::result::Result<(Transaction, u64, Report), Report>> {
    if height().is_none_or(|h| h <= last_valid) {
        return Ok(Ok((tx, last_valid, report)));
    }
    match after_approval(chain, payload, signers, payer, report)? {
        Ok((fresh, last_valid, r)) => match height() {
            Some(h) if h > last_valid => Ok(Err(r.refused(&refusal(
                "blockhash",
                format!(
                    "the RPC's blockhash is already expired (finalized height {h} is past {last_valid}), also after a fresh one: the RPC lags; nothing was sent"
                ),
            )))),
            _ => Ok(Ok((fresh, last_valid, r))),
        },
        Err(failed) => Ok(Err(failed)),
    }
}

/// ed25519 signatures are deterministic: the same instructions with the same blockhash give the
/// same signature, so a repeated payload would resend the first transaction and report its outcome
/// as its own. A signature already in the audit log is re-signed and simulated once with a fresh
/// blockhash; still a repeat, it is refused and nothing is sent, so a retry is safe.
#[expect(
    clippy::too_many_arguments,
    reason = "the signed transaction, its signing inputs and the report; a struct would serve only the lint"
)]
pub fn unless_duplicate(
    chain: &impl Fresh,
    seen: impl Fn(&str) -> bool,
    tx: Transaction,
    last_valid: u64,
    payload: &Payload,
    signers: &[&Keypair],
    payer: &solana_address::Address,
    report: Report,
) -> Result<std::result::Result<(Transaction, u64, Report), Report>> {
    let repeat = |tx: &Transaction| tx.signatures.first().is_some_and(|s| seen(&s.to_string()));
    if !repeat(&tx) {
        return Ok(Ok((tx, last_valid, report)));
    }
    match after_approval(chain, payload, signers, payer, report)? {
        Ok((fresh, _, r)) if repeat(&fresh) => Ok(Err(r.refused(&refusal(
            "blockhash",
            "this transaction's signature is already in the audit log (the same instructions with the same blockhash), and the RPC returned that blockhash again: nothing was sent, and a retry is safe"
                .to_owned(),
        )))),
        Ok(fresh) => Ok(Ok(fresh)),
        Err(failed) => Ok(Err(failed)),
    }
}

/// Whether `signature` is in this key's audit entries (intents or finals) from the last 24 h.
pub fn seen(entries: &[Entry], key: &str, signature: &str, now: u64) -> bool {
    entries.iter().any(|e| {
        e.key == key && e.ts >= now.saturating_sub(DAY) && e.signature.as_deref() == Some(signature)
    })
}

/// Re-signs the payload's instructions with a fresh blockhash and simulates that exact message,
/// unsigned. Returns the transaction, its last valid block height, and its own simulation, so
/// what is sent is always what was simulated.
pub fn fresh_simulated(
    chain: &impl Fresh,
    payload: &Payload,
    signers: &[&Keypair],
    payer: &solana_address::Address,
) -> Result<(Transaction, u64, Simulation)> {
    let (hash, last_valid) = chain.latest_blockhash()?;
    let message =
        Message::new_with_blockhash(&payload.instructions, Some(&payload.fee_payer), &hash);
    let tx = build(message, signers, hash)?;
    let sim = chain.simulate_unsigned(&encode_unsigned(&tx)?, &[*payer])?;
    Ok((tx, last_valid, sim))
}
