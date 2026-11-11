//! The check and sign pipeline.
//!
//! This module is the report, the checks and the pipeline's order; `fresh` holds the gates that may
//! rebuild the transaction, `approval` the terminal prompt, and `send` the wait for an outcome.

mod approval;
mod fresh;
mod send;

pub use send::CONFIRM_DEADLINE;

use fresh::{approved_fresh, fresh_simulated, seen, unless_duplicate};
use send::{POLL, send_and_confirm};

use std::io::Read;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde::Serialize;
use serde_json::Value;
use solana_hash::Hash;
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use zeroize::Zeroizing;

use crate::audit::{Audit, Entry, Finding, KeyLock};
use crate::checks::{Refusal, refuse as refusal};
use crate::cluster::Cluster;
use crate::config::{Approval, Config, KeyConfig};
use crate::facts;
use crate::halt;
use crate::instruction_table::InstructionTable;
use crate::keystore::KeyStore;
use crate::payload::Payload;
use crate::policy::{self, Facts, KeyPolicy, Request, Stage, Verdict};
use crate::rpc::{Rpc, SimBlockhash, delta};
use crate::text;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// `check` only: every check and the simulation passed.
    Ok,
    Refused,
    SimulationFailed,
    Confirmed,
    /// Landed with a program error.
    Failed,
    /// Sent but not proven landed. This includes a transaction whose blockhash has expired with
    /// no status found: through an RPC that may answer from several nodes, absence is never
    /// proof that it did not land. Reconcile the signature and the on-chain state before retrying.
    Unknown,
}

impl Outcome {
    pub fn exit_code(self) -> ExitCode {
        ExitCode::from(match self {
            Self::Ok | Self::Confirmed => 0,
            Self::Refused => 10,
            Self::SimulationFailed => 11,
            Self::Failed => 12,
            // 13 was `expired`, which the signer no longer reports (see `Unknown`).
            Self::Unknown => 14,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub outcome: Outcome,
    pub key: Option<String>,
    pub signature: Option<String>,
    pub slot: Option<u64>,
    pub explorer: Option<String>,
    pub detail: Option<String>,
    /// The check that refused the payload.
    pub failed_check: Option<&'static str>,
    /// The program's own error message, from the logs.
    pub program_error: Option<String>,
    pub error: Option<Value>,
    pub checks: Vec<&'static str>,
    /// `check` only: what `sign` would refuse now that `check` does not.
    pub sign_preview: Option<SignPreview>,
    /// The instruction table's findings, in report mode: never a refusal.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Refusal>,
    pub compute_units: Option<u64>,
    pub fee: Option<u64>,
    pub balance_change: Option<i64>,
    /// Builder-supplied, not verified.
    pub summary: String,
    pub warnings: Vec<String>,
    pub payload_hash: String,
    pub logs: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SignPreview {
    /// The sign-only checks that would refuse, in order.
    pub refusals: Vec<Refusal>,
    /// A `confirm` key: `sign` waits for a person at the terminal.
    pub needs_approval: bool,
}

impl Report {
    fn new(payload: &Payload, key: &KeyConfig) -> Self {
        Self {
            outcome: Outcome::Ok,
            key: Some(key.name.clone()),
            signature: None,
            slot: None,
            explorer: None,
            detail: None,
            failed_check: None,
            program_error: None,
            error: None,
            checks: Vec::new(),
            sign_preview: None,
            findings: Vec::new(),
            compute_units: None,
            fee: None,
            balance_change: None,
            summary: payload.summary.clone(),
            warnings: payload.warnings.clone(),
            payload_hash: payload.hash.clone(),
            logs: Vec::new(),
        }
    }

    fn refused(mut self, r: &Refusal) -> Self {
        self.outcome = Outcome::Refused;
        self.detail = Some(r.to_string());
        self.failed_check = Some(r.check);
        self
    }
}

pub enum Mode {
    Check,
    Sign { intent: Option<String> },
}

/// Audit command of the entry written before a transaction is sent.
pub const SIGN_INTENT: &str = "sign-intent";

pub fn run(cfg: &Config, key_name: Option<&str>, payload_arg: &str, mode: &Mode) -> Result<Report> {
    let text = read_payload(payload_arg)?;
    let (report, sent, secrets) = execute(cfg, key_name, &text, mode, None)?;
    drop(text);
    // Payloads with partial signers hold secrets; remove them once sent.
    if sent && secrets && payload_arg != "-" {
        std::fs::remove_file(payload_arg).with_context(|| format!("removing {payload_arg}"))?;
    }
    Ok(report)
}

/// Checks and, in sign mode, signs payload JSON. `allowed` limits the usable keys (a serve token).
/// Returns the report, whether it was sent, and whether the payload carried partial signers.
pub fn execute(
    cfg: &Config,
    key_name: Option<&str>,
    text: &str,
    mode: &Mode,
    allowed: Option<&[String]>,
) -> Result<(Report, bool, bool)> {
    let payload = Payload::parse(text)?;
    let key = match key_name {
        Some(n) => cfg.key(n)?,
        None => cfg.key_for_pubkey(&payload.fee_payer)?,
    };
    let signing = matches!(mode, Mode::Sign { .. });
    let span = if signing {
        tracing::info_span!("sign", key = %key.name, payload = %payload.hash)
    } else {
        tracing::info_span!("check", key = %key.name, payload = %payload.hash)
    };
    let _span = span.entered();
    let authorized = allowed.is_none_or(|keys| keys.contains(&key.name));
    // Held until this function returns, after the final audit entry: no other sign with this key,
    // in this process or another, can pass the limit check in between. Not taken for a token that
    // may not use the key, so it cannot queue behind (or hold up) that key's signs.
    let _lock = if signing && authorized {
        let _wait = tracing::info_span!("key_lock").entered();
        Some(KeyLock::acquire(&cfg.state_dir, &key.name)?)
    } else {
        None
    };
    let (mut report, sent) = if authorized {
        pipeline(cfg, key, &payload, mode)?
    } else {
        let r = refusal(
            "authorized",
            format!("this token may not use key {:?}", key.name),
        );
        (Report::new(&payload, key).refused(&r), false)
    };
    if signing {
        let entry = Entry {
            ts: now(),
            key: key.name.clone(),
            command: "sign".to_owned(),
            payload_hash: payload.hash.clone(),
            programs: program_ids(&payload),
            summary: text::clip(&payload.summary),
            intent: logged_intent(mode),
            outcome: serde_json::to_value(report.outcome)?
                .as_str()
                .unwrap_or_default()
                .to_owned(),
            detail: report.detail.clone(),
            signature: report.signature.clone(),
            slot: report.slot,
            balance_change: report.balance_change,
            findings: report
                .findings
                .iter()
                .map(|f| Finding {
                    check: f.check.to_owned(),
                    detail: f.detail.clone(),
                })
                .collect(),
            policy: Some(key.policy.clone()),
            ..Entry::default()
        };
        Audit::new(&cfg.state_dir).append(entry)?;
        if report.outcome == Outcome::Unknown {
            after_unknown(cfg, key, &mut report);
        }
    }
    tracing::info!(
        outcome = serde_json::to_value(report.outcome)?
            .as_str()
            .unwrap_or_default(),
        failed_check = report.failed_check,
        sent,
        "done"
    );
    Ok((report, sent, !payload.partial_signers.is_empty()))
}

/// Under the key lock, after an `unknown` sign is logged: halts the key once its streak reaches
/// `halt_after_unknown`, and says so in the report. The sign's own outcome stands either way.
fn after_unknown(cfg: &Config, key: &KeyConfig, report: &mut Report) {
    let note = match halt::trip_after_unknown(
        &cfg.state_dir,
        &key.name,
        key.halt_after_unknown,
        now(),
    ) {
        Ok(true) => format!(
            "key {:?} is now halted after {} consecutive unknown outcomes; reconcile them, then `sa-forge-signer resume --key {}` at a terminal",
            key.name, key.halt_after_unknown, key.name
        ),
        Ok(false) => return,
        Err(e) => format!("could not check or write the halt_after_unknown halt: {e:#}"),
    };
    report.detail = Some(match report.detail.take() {
        Some(d) => format!("{d}; {note}"),
        None => note,
    });
}

/// The inputs of `policy::evaluate` for one payload and key, and the facts read so far. Kept from
/// the checks before simulation to the cap after it, so both use the same function.
struct Gate<'a> {
    key: &'a KeyConfig,
    cluster: &'static Cluster,
    allowed: Vec<solana_address::Address>,
    games: Vec<solana_address::Address>,
    fee_payer: solana_address::Address,
    partial: Vec<solana_address::Address>,
    /// Compiled without a blockhash: no check depends on it.
    message: Message,
    facts: Facts,
    /// This key's recent audit entries, read once. Under the key lock (in `sign`) no other sign
    /// can add to them, so the checks before sending reuse them.
    entries: Vec<Entry>,
    table: Option<&'a InstructionTable>,
}

impl<'a> Gate<'a> {
    fn new(cfg: &'a Config, key: &'a KeyConfig, payload: &Payload) -> Result<Self> {
        let now = now();
        let read = tracing::info_span!("audit_read", entries = tracing::field::Empty).entered();
        let mut entries = Audit::new(&cfg.state_dir).recent(now)?;
        read.record("entries", entries.len());
        drop(read);
        entries.retain(|e| e.key == key.name);
        Ok(Self {
            key,
            cluster: cfg.cluster,
            allowed: cfg.allowed_programs(key)?,
            games: cfg.game_programs()?,
            fee_payer: payload.fee_payer,
            partial: payload.partial_pubkeys(),
            message: Message::new(&payload.instructions, Some(&payload.fee_payer)),
            facts: facts::local(&cfg.state_dir, key, &entries, now)?,
            entries,
            table: cfg.instruction_table.as_ref(),
        })
    }

    fn evaluate(&self, stage: Stage) -> Verdict {
        let request = Request {
            fee_payer: &self.fee_payer,
            message: &self.message,
            partial_signers: &self.partial,
        };
        let policy = KeyPolicy {
            key: self.key,
            cluster: self.cluster,
            allowed_programs: &self.allowed,
            game_programs: &self.games,
            table: self.table,
        };
        policy::evaluate(&request, &self.facts, &policy, stage)
    }

    /// Reads the RPC facts one at a time, in check order, until a refusal is decided or all are
    /// read: a refusal the earlier checks decide is made before the next RPC call.
    fn decide(&mut self, rpc: &Rpc, stage: Stage) -> Result<Verdict> {
        loop {
            let verdict = self.evaluate(stage);
            if verdict.refusal().is_some() {
                return Ok(verdict);
            }
            let request = Request {
                fee_payer: &self.fee_payer,
                message: &self.message,
                partial_signers: &self.partial,
            };
            let policy = KeyPolicy {
                key: self.key,
                cluster: self.cluster,
                allowed_programs: &self.allowed,
                game_programs: &self.games,
                table: self.table,
            };
            if !facts::read_next(&mut self.facts, rpc, &request, &policy)? {
                return Ok(verdict);
            }
        }
    }
}

/// `check` only: what `sign` would refuse now that `check` does not, from `verdict`.
fn with_preview(mut report: Report, verdict: &Verdict, key: &KeyConfig, mode: &Mode) -> Report {
    if matches!(mode, Mode::Check) {
        report.sign_preview = Some(SignPreview {
            refusals: verdict.sign_only_failures(),
            needs_approval: key.approval == Approval::Confirm,
        });
    }
    report
}

fn pipeline(
    cfg: &Config,
    key: &KeyConfig,
    payload: &Payload,
    mode: &Mode,
) -> Result<(Report, bool)> {
    let mut report = Report::new(payload, key);
    let signing = matches!(mode, Mode::Sign { .. });
    let rpc = Rpc::new(&cfg.rpc_url);
    let mut gate = Gate::new(cfg, key, payload)?;
    let verdict = gate.decide(
        &rpc,
        if signing {
            Stage::BeforeSimulation
        } else {
            Stage::Check
        },
    )?;
    report.checks = verdict.passed();
    report.findings.clone_from(&verdict.findings);
    if let Some(r) = verdict.refusal() {
        return Ok((with_preview(report.refused(r), &verdict, key, mode), false));
    }
    let (blockhash, mut last_valid) = rpc.latest_blockhash()?;
    let message =
        Message::new_with_blockhash(&payload.instructions, Some(&payload.fee_payer), &blockhash);

    let keypair = if signing {
        Some(load_key(cfg, key)?)
    } else {
        None
    };
    let signers = signers_for(keypair.as_ref(), &payload.partial_signers);
    let mut tx = build(message, &signers, blockhash)?;
    let pre = rpc.balance(&key.pubkey)?;
    // Unsigned, with its own blockhash when signing: no node holds a signed copy before the intent.
    let sim_blockhash = if signing {
        SimBlockhash::Keep
    } else {
        SimBlockhash::Replace
    };
    let mut sim = rpc.simulate(&encode_unsigned(&tx)?, sim_blockhash, &[key.pubkey])?;
    // A slow preflight can outlive the blockhash on the simulating node; every key is here, so re-sign once.
    if signing && sim.err.as_ref().and_then(Value::as_str) == Some("BlockhashNotFound") {
        (tx, last_valid, sim) = fresh_simulated(&rpc, payload, &signers, &key.pubkey)?;
    }
    report.compute_units = sim.units;
    // The fee of the transaction actually built, which a blockhash rebuild replaces.
    report.fee = rpc
        .fee_for_message(&b64(&tx.message.serialize()))
        .ok()
        .flatten();
    report.balance_change = sim
        .post_lamports
        .first()
        .copied()
        .flatten()
        .and_then(|post| delta(pre, post));
    if let Some(err) = sim.err {
        report.outcome = Outcome::SimulationFailed;
        report.program_error = program_error(&sim.logs);
        report.detail.clone_from(&report.program_error);
        report.error = Some(err);
        report.logs = sim.logs;
        return Ok((with_preview(report, &verdict, key, mode), false));
    }
    report.checks.push("simulation");
    if !signing {
        report.logs = sim.logs;
        // The cap as `sign` would check it, against this simulation.
        let after = gate.evaluate(Stage::AfterSimulation {
            reserve: reservation(&report),
        });
        return Ok((with_preview(report, &after, key, mode), false));
    }

    let (tx, last_valid) =
        match approved_fresh(&rpc, key, payload, &signers, tx, last_valid, report)? {
            Ok((tx, last_valid, r)) => {
                report = r;
                (tx, last_valid)
            }
            Err(failed) => return Ok((failed, false)),
        };
    let (tx, last_valid, reserve, report) =
        match unique_within_cap(&rpc, &mut gate, payload, &signers, tx, last_valid, report)? {
            Ok(v) => v,
            Err(r) => return Ok((r, false)),
        };
    // Durable before anything is sent: if this process dies mid-confirmation, the intent (with
    // this transaction's reservation) is what the limits and the operator see.
    Audit::new(&cfg.state_dir).append(intent_entry(key, payload, mode, &tx, reserve))?;
    let explorer = &cfg.explorer_url;
    send_and_confirm(
        &rpc,
        &tx,
        last_valid,
        report,
        explorer,
        POLL,
        CONFIRM_DEADLINE,
    )
    .map(|r| (r, true))
}

/// The `sign-intent` entry written before `tx` is sent, with its reservation.
fn intent_entry(
    key: &KeyConfig,
    payload: &Payload,
    mode: &Mode,
    tx: &Transaction,
    reserve: u64,
) -> Entry {
    Entry {
        ts: now(),
        key: key.name.clone(),
        command: SIGN_INTENT.to_owned(),
        payload_hash: payload.hash.clone(),
        programs: program_ids(payload),
        summary: text::clip(&payload.summary),
        intent: logged_intent(mode),
        outcome: "pending".to_owned(),
        signature: tx.signatures.first().map(ToString::to_string),
        balance_change: Some(0_i64.saturating_sub_unsigned(reserve)),
        policy: Some(key.policy.clone()),
        ..Entry::default()
    }
}

/// The caller's `intent`, as an audit entry keeps it.
fn logged_intent(mode: &Mode) -> Option<String> {
    match mode {
        Mode::Sign { intent } => intent.as_deref().map(text::clip),
        Mode::Check => None,
    }
}

/// The last gates before the intent, under the key lock: the duplicate-signature guard, then the
/// cap against the simulation of the exact transaction about to be sent (what is already spent or
/// reserved plus its own spend). Returns that transaction, its last valid height and its reservation.
fn unique_within_cap(
    rpc: &Rpc,
    gate: &mut Gate<'_>,
    payload: &Payload,
    signers: &[&Keypair],
    tx: Transaction,
    last_valid: u64,
    report: Report,
) -> Result<std::result::Result<(Transaction, u64, u64, Report), Report>> {
    let key = gate.key;
    let now = now();
    let entries = std::mem::take(&mut gate.entries);
    let logged = |s: &str| seen(&entries, &key.name, s, now);
    let (tx, last_valid, mut report) = match unless_duplicate(
        rpc,
        logged,
        tx,
        last_valid,
        payload,
        signers,
        &key.pubkey,
        report,
    )? {
        Ok((tx, last_valid, mut r)) => {
            r.checks.push("duplicate");
            (tx, last_valid, r)
        }
        Err(failed) => return Ok(Err(failed)),
    };
    // The same evaluation as before simulation, now with this transaction's reservation and
    // the usage as of now; the earlier checks passed already and their facts are unchanged.
    gate.facts.usage = facts::usage(&entries, key, now);
    let reserve = reservation(&report);
    if let Some(r) = gate.evaluate(Stage::AfterSimulation { reserve }).refusal() {
        return Ok(Err(report.refused(r)));
    }
    let Some(reserve) = reserve else {
        return Ok(Err(report.refused(&policy::unbounded())));
    };
    report.checks.push("cap");
    Ok(Ok((tx, last_valid, reserve, report)))
}

/// Signing uses the key and the payload's partial signers; `check` has no key and simulates unsigned.
fn signers_for<'a>(key: Option<&'a Keypair>, partial: &'a [Keypair]) -> Vec<&'a Keypair> {
    key.map_or_else(Vec::new, |k| std::iter::once(k).chain(partial).collect())
}

fn build(message: Message, signers: &[&Keypair], blockhash: Hash) -> Result<Transaction> {
    let mut tx = Transaction::new_unsigned(message);
    if !signers.is_empty() {
        tx.try_sign(signers, blockhash).context("signing")?;
    }
    Ok(tx)
}

fn load_key(cfg: &Config, key: &KeyConfig) -> Result<Keypair> {
    let kp = KeyStore::new(&cfg.key_dir).load(&key.name)?;
    if kp.pubkey() != key.pubkey {
        bail!(
            "key file for {:?} has pubkey {}, but the config says {}",
            key.name,
            kp.pubkey(),
            key.pubkey
        );
    }
    Ok(kp)
}

/// This transaction's reservation: its simulated spend plus its fee. Both must be known: the
/// simulated balance usually already includes the fee, so adding it again over-reserves by one
/// fee, but a missing fee is never assumed to be zero.
fn reservation(report: &Report) -> Option<u64> {
    Some(facts::spend(report.balance_change?).saturating_add(report.fee?))
}

fn read_payload(arg: &str) -> Result<Zeroizing<String>> {
    let mut text = Zeroizing::new(String::new());
    if arg == "-" {
        std::io::stdin()
            .read_to_string(&mut text)
            .context("reading the payload from stdin")?;
    } else {
        let meta = std::fs::symlink_metadata(arg).with_context(|| format!("reading {arg}"))?;
        if !meta.file_type().is_file() {
            bail!("{arg} is not a regular file");
        }
        std::fs::File::open(arg)?.read_to_string(&mut text)?;
    }
    Ok(text)
}

fn program_ids(payload: &Payload) -> Vec<String> {
    let mut ids: Vec<String> = payload
        .instructions
        .iter()
        .map(|i| i.program_id.to_string())
        .collect();
    ids.dedup();
    ids
}

fn encode(tx: &Transaction) -> Result<String> {
    Ok(b64(
        &wincode::serialize(tx).map_err(|e| anyhow::anyhow!("serializing: {e}"))?
    ))
}

/// `tx` with its signatures zeroed: the form a node simulates, which it cannot land.
fn encode_unsigned(tx: &Transaction) -> Result<String> {
    encode(&Transaction::new_unsigned(tx.message.clone()))
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The program's own message: a logged error, else the runtime's "failed:" line.
fn program_error(logs: &[String]) -> Option<String> {
    logs.iter()
        .find_map(|l| {
            l.strip_prefix("Program log: ")
                .filter(|m| m.contains("Error"))
                .map(str::to_owned)
        })
        .or_else(|| logs.iter().rev().find(|l| l.contains(" failed: ")).cloned())
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests;
