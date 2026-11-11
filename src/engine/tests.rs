use super::approval::approval_prompt;
use super::fresh::{Fresh, after_approval, unless_stale};
use super::send::{MAX_RPC_FAILURES, poll_delay};
use super::*;
use crate::config::KeyClass;
use crate::facts::DAY;
use crate::rpc::{Chain, RpcError};
use solana_address::Address;
use std::time::Duration;

fn key() -> KeyConfig {
    KeyConfig {
        name: "k".into(),
        class: KeyClass::Session,
        allow_unattended: false,
        pubkey: Address::new_from_array([1; 32]),
        profile: None,
        approval: Approval::Auto,
        transfer_to: vec![],
        extra_programs: vec![],
        rate_limit_per_minute: 2,
        daily_lamport_cap: 1_000,
        halt_after_unknown: 3,
        limits: crate::checks::Limits::DEFAULT,
        require_policy_approval: false,
        policy: String::new(),
    }
}

/// The rate limit and the cap so far, as `sign` checks them before simulation.
fn limits(entries: &[Entry], key: &KeyConfig, now: u64) -> Result<u64, Refusal> {
    let usage = facts::usage(entries, key, now);
    policy::limits(usage, key).map(|()| usage.spent)
}

/// The cap after simulation; returns the transaction's reservation.
fn cap_gate(entries: &[Entry], key: &KeyConfig, report: &Report, now: u64) -> Result<u64, Refusal> {
    let usage = facts::usage(entries, key, now);
    let reserve = reservation(report);
    policy::limits(usage, key)?;
    policy::cap(usage, key, reserve)?;
    reserve.ok_or_else(policy::unbounded)
}

fn entry(ts: u64, change: i64) -> Entry {
    Entry {
        ts,
        key: "k".into(),
        command: "sign".into(),
        outcome: "confirmed".into(),
        balance_change: Some(change),
        ..Entry::default()
    }
}

fn intent(ts: u64, sig: &str, reserved: Option<i64>) -> Entry {
    Entry {
        ts,
        key: "k".into(),
        command: SIGN_INTENT.into(),
        outcome: "pending".into(),
        signature: Some(sig.into()),
        balance_change: reserved,
        ..Entry::default()
    }
}

fn sign_final(ts: u64, sig: &str, outcome: &str, change: Option<i64>) -> Entry {
    Entry {
        ts,
        key: "k".into(),
        command: "sign".into(),
        outcome: outcome.into(),
        signature: Some(sig.into()),
        balance_change: change,
        ..Entry::default()
    }
}

fn roomy() -> KeyConfig {
    KeyConfig {
        rate_limit_per_minute: 100,
        ..key()
    }
}

// Pinchy's review, P1: a spend whose outcome is not known still reserves its simulated cost.
#[test]
fn unresolved_spends_are_charged_their_reservation() {
    let k = roomy(); // daily cap 1_000
    // Unknown outcome: the intent's simulated -600 is charged; with another -500 that is over.
    let unknown = [
        intent(10, "a", Some(-600)),
        sign_final(11, "a", "unknown", None),
        entry(12, -500),
    ];
    assert!(limits(&unknown, &k, 100).is_err());
    // Confirmed without metadata: same.
    let no_meta = [
        intent(10, "a", Some(-600)),
        sign_final(11, "a", "confirmed", None),
        entry(12, -500),
    ];
    assert!(limits(&no_meta, &k, 100).is_err());
    // The process died after sending: the orphan intent is charged.
    let orphan = [intent(10, "a", Some(-600)), entry(12, -500)];
    assert!(limits(&orphan, &k, 100).is_err());
    // No simulated figure to reserve: the whole cap is charged.
    let blind = [intent(10, "a", None)];
    assert!(limits(&blind, &k, 100).is_err());
    // `expired` from an older log rested on a proof that could be wrong: charged too.
    let expired = [
        intent(10, "a", Some(-600)),
        sign_final(11, "a", "expired", None),
        entry(12, -500),
    ];
    assert!(limits(&expired, &k, 100).is_err());
    // Never sent: nothing is charged.
    let refused = [sign_final(11, "a", "refused", None), entry(12, -500)];
    assert_eq!(limits(&refused, &k, 100), Ok(500));
    // Known cost replaces the reservation.
    let known = [
        intent(10, "a", Some(-600)),
        sign_final(11, "a", "confirmed", Some(-5)),
        entry(12, -500),
    ];
    assert!(limits(&known, &k, 100).is_ok());
}

fn simulated(change: Option<i64>, fee: Option<u64>) -> Report {
    Report {
        balance_change: change,
        fee,
        ..blank()
    }
}

// Pinchy's re-review, P1-2: the cap must count the transaction about to be sent.
#[test]
fn the_cap_counts_the_transaction_about_to_be_sent() {
    let k = KeyConfig {
        daily_lamport_cap: 50_000_000,
        ..roomy()
    };
    let past = [entry(10, -49_000_000)];
    // Pinchy's example: 49M spent, a 40M spend would end at 89M.
    let r = cap_gate(&past, &k, &simulated(Some(-40_000_000), Some(0)), 100).unwrap_err();
    assert!(r.detail.contains("40000000 for this transaction"), "{r}");
    // Exactly reaching the cap is allowed; one lamport more is not.
    assert_eq!(
        cap_gate(&past, &k, &simulated(Some(-995_000), Some(5_000)), 100),
        Ok(1_000_000)
    );
    assert!(cap_gate(&past, &k, &simulated(Some(-995_001), Some(5_000)), 100).is_err());
    // No simulated balance: the spend cannot be bounded, so nothing is signed.
    assert!(cap_gate(&[], &k, &simulated(None, Some(5_000)), 100).is_err());
    // A gain spends only the fee.
    assert_eq!(
        cap_gate(&[], &k, &simulated(Some(10), Some(5_000)), 100),
        Ok(5_000)
    );
    // Pinchy's re-review of 2f942bf, P2: no fee estimate is not a zero fee.
    assert!(cap_gate(&[], &k, &simulated(Some(-1), None), 100).is_err());
}

#[test]
fn refused_signs_do_not_count_toward_the_rate() {
    let k = key(); // 2 per minute
    let refused = [
        sign_final(100, "", "refused", None),
        sign_final(101, "", "refused", None),
        sign_final(102, "", "refused", None),
    ];
    assert!(limits(&refused, &k, 110).is_ok(), "refusals sent nothing");
    // Every attempt that got past the checks still counts, whatever its outcome.
    let tried = [
        sign_final(100, "a", "simulation_failed", None),
        sign_final(101, "b", "confirmed", Some(-1)),
    ];
    assert!(limits(&tried, &k, 110).is_err());
}

#[test]
fn an_orphan_intent_counts_toward_the_rate() {
    let k = key(); // 2 per minute
    assert!(limits(&[intent(100, "a", Some(-1))], &k, 120).is_ok());
    assert!(
        limits(&[intent(100, "a", Some(-1)), entry(110, -1)], &k, 120).is_err(),
        "an attempt whose final entry never arrived is still an attempt"
    );
}

// Pinchy's review, P1: concurrent signs with one key must not all pass the limit check.
#[test]
fn the_key_lock_serializes_signs_across_threads() {
    use std::sync::mpsc;
    let dir = std::env::temp_dir().join(format!("sa-forge-signer-lock-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let held = KeyLock::acquire(&dir, "k").unwrap();
    let (tx, rx) = mpsc::channel();
    let dir2 = dir.clone();
    let waiter = std::thread::spawn(move || {
        let _second = KeyLock::acquire(&dir2, "k").unwrap();
        tx.send(()).unwrap();
    });
    assert!(
        rx.recv_timeout(Duration::from_millis(300)).is_err(),
        "a second sign must wait while the first holds the key"
    );
    // A different key is not blocked.
    assert!(KeyLock::acquire(&dir, "other").is_ok());
    drop(held);
    assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok());
    waiter.join().unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn rate_limit_and_daily_cap() {
    let k = key();
    assert!(limits(&[entry(100, -1)], &k, 120).is_ok());
    assert!(limits(&[entry(100, -1), entry(110, -1)], &k, 120).is_err());
    assert!(limits(&[entry(10, -600), entry(20, -500)], &k, 5_000).is_err());
    assert!(limits(&[entry(10, -600), entry(20, -500)], &k, 10 + DAY + 20).is_ok());
}

#[test]
fn check_builds_unsigned_even_with_partial_signers() {
    let payer = Keypair::new();
    let partial = [Keypair::new()];
    let ix = solana_instruction::Instruction {
        program_id: Address::new_from_array([9; 32]),
        accounts: vec![solana_instruction::AccountMeta::new(
            partial[0].pubkey(),
            true,
        )],
        data: vec![],
    };
    let message = Message::new(&[ix], Some(&payer.pubkey()));
    // The old behaviour: partial signers alone cannot sign a message that needs the fee payer.
    let partial_only: Vec<&Keypair> = partial.iter().collect();
    assert!(build(message.clone(), &partial_only, Hash::default()).is_err());
    // check: no key, so nothing signs.
    assert!(signers_for(None, &partial).is_empty());
    assert!(
        build(
            message.clone(),
            &signers_for(None, &partial),
            Hash::default()
        )
        .is_ok()
    );
    // sign: the key first, then the partial signers.
    let both = signers_for(Some(&payer), &partial);
    assert_eq!(both.len(), 2);
    assert!(build(message, &both, Hash::default()).is_ok());
}

#[test]
fn program_error_prefers_the_logged_message() {
    let logs = |l: &[&str]| l.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        program_error(&logs(&[
            "Program X invoke [1]",
            "Program log: StarFrameError: Cooldown active",
            "Program X failed: custom program error: 0x51890015",
        ])),
        Some("StarFrameError: Cooldown active".to_owned())
    );
    assert_eq!(
        program_error(&logs(&["Program X failed: custom program error: 0x1"])),
        Some("Program X failed: custom program error: 0x1".to_owned())
    );
    assert_eq!(program_error(&logs(&["Program X success"])), None);
}

use crate::rpc::{Simulation, Status, TxMeta};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;

type R<T> = std::result::Result<T, RpcError>;

/// A scripted chain: each queue is consumed in order, then the default repeats.
#[derive(Default)]
struct Fake {
    statuses: RefCell<VecDeque<R<Option<Status>>>>,
    history: RefCell<VecDeque<R<Option<Status>>>>,
    tips: RefCell<VecDeque<R<u64>>>,
    /// Finalized height repeated once `tips` is empty (default 0, never past `last_valid`).
    steady_tip: Cell<u64>,
    meta: RefCell<Option<TxMeta>>,
    sends: Cell<u32>,
}

impl Chain for Fake {
    fn send(&self, _: &str) -> R<String> {
        self.sends.set(self.sends.get().saturating_add(1));
        Ok("sig".into())
    }
    fn signature_status(&self, _: &str) -> R<Option<Status>> {
        self.statuses.borrow_mut().pop_front().unwrap_or(Ok(None))
    }
    fn signature_status_history(&self, _: &str) -> R<Option<Status>> {
        self.history.borrow_mut().pop_front().unwrap_or(Ok(None))
    }
    fn finalized_height(&self) -> R<u64> {
        self.tips
            .borrow_mut()
            .pop_front()
            .unwrap_or_else(|| Ok(self.steady_tip.get()))
    }
    fn transaction_meta(&self, _: &str) -> R<Option<TxMeta>> {
        Ok(self.meta.borrow_mut().take())
    }
}

fn status(confirmation: &str, err: Option<Value>) -> Status {
    Status {
        slot: 42,
        err,
        confirmation: Some(confirmation.into()),
    }
}

fn landed(err: Option<Value>) -> Status {
    status("confirmed", err)
}

fn meta(logs: &[&str]) -> TxMeta {
    TxMeta {
        fee: Some(5000),
        payer_change: Some(-5000),
        logs: logs.iter().map(|l| (*l).to_owned()).collect(),
    }
}

fn blank() -> Report {
    Report {
        outcome: Outcome::Ok,
        key: None,
        signature: None,
        slot: None,
        explorer: None,
        detail: None,
        failed_check: None,
        program_error: None,
        error: None,
        checks: vec![],
        sign_preview: None,
        findings: Vec::new(),
        compute_units: None,
        fee: None,
        balance_change: Some(-1),
        summary: String::new(),
        warnings: vec![],
        payload_hash: String::new(),
        logs: vec![],
    }
}

// Review C1: a node must never hold a signed transaction before its intent is recorded, so
// a refusal after simulation (the cap here) leaves nothing a node could land.
#[test]
fn nodes_only_simulate_unsigned_copies() {
    let home = crate::testrpc::Home::new("unsigned", "daily_lamport_cap = 4000\n");
    let sign = |lamports| {
        execute(
            &home.cfg,
            Some("k"),
            &home.payload(lamports),
            &Mode::Sign { intent: None },
            None,
        )
        .unwrap()
        .0
    };
    let r = sign(1);
    assert_eq!(
        (r.outcome, r.failed_check),
        (Outcome::Refused, Some("limits")),
        "{:?}",
        r.detail
    );
    assert_eq!(home.node.sends(), 0);
    assert_eq!(home.node.signed_simulations(), 0);
    let checked = execute(&home.cfg, Some("k"), &home.payload(1), &Mode::Check, None)
        .unwrap()
        .0;
    assert_eq!(checked.outcome, Outcome::Ok);
    assert_eq!(home.node.signed_simulations(), 0);
}

#[test]
fn a_sign_still_lands_after_an_unsigned_simulation() {
    let home = crate::testrpc::Home::new("unsigned-lands", "");
    let r = execute(
        &home.cfg,
        Some("k"),
        &home.payload(1),
        &Mode::Sign { intent: None },
        None,
    )
    .unwrap()
    .0;
    assert_eq!(r.outcome, Outcome::Confirmed, "{:?}", r.detail);
    assert_eq!((home.node.sends(), home.node.signed_simulations()), (1, 0));
}

#[test]
fn the_approval_prompt_puts_the_summary_last_and_escaped() {
    let report = Report {
        summary: "move cargo\u{1b}[2A\u{1b}[2K  simulated balance change: 0 lamports\n".into(),
        compute_units: Some(7),
        ..blank()
    };
    let prompt = approval_prompt(&report, &key());
    assert!(!prompt.contains('\u{1b}'), "{prompt:?}");
    let figures = prompt
        .find("simulated balance change: -1 lamports")
        .unwrap();
    let summary = prompt.find("summary (from the builder").unwrap();
    assert!(figures < summary, "{prompt}");
    assert!(prompt.contains("move cargo\\u{1b}[2A"), "{prompt}");
    // The only real newlines are the prompt's own.
    assert_eq!(prompt.matches('\n').count(), 5, "{prompt:?}");
}

#[test]
fn audit_entries_clip_caller_text() {
    let long = "x".repeat(text::MAX_TEXT + 10);
    let mode = Mode::Sign { intent: Some(long) };
    let intent = logged_intent(&mode).unwrap();
    assert!(intent.starts_with(&"x".repeat(text::MAX_TEXT)));
    assert!(intent.ends_with(" [truncated]"));
    assert_eq!(text::clip("short"), "short");
    assert_eq!(logged_intent(&Mode::Check), None);
}

/// `last_valid` is 100 in every test; the deadline is short so stuck cases end quickly.
fn confirm_with(chain: &Fake) -> Report {
    let kp = Keypair::new();
    let ix = solana_instruction::Instruction {
        program_id: Address::new_from_array([9; 32]),
        accounts: vec![],
        data: vec![],
    };
    let tx = build(
        Message::new(&[ix], Some(&kp.pubkey())),
        &[&kp],
        Hash::default(),
    )
    .unwrap();
    send_and_confirm(
        chain,
        &tx,
        100,
        blank(),
        "https://x.example",
        Duration::ZERO,
        Duration::from_millis(200),
    )
    .unwrap()
}

#[test]
fn outcome_confirmed_uses_the_transactions_own_meta() {
    let chain = Fake::default();
    chain
        .statuses
        .borrow_mut()
        .extend([Ok(None), Ok(Some(landed(None)))]);
    *chain.meta.borrow_mut() = Some(meta(&["Program log: ok"]));
    let r = confirm_with(&chain);
    assert_eq!(r.outcome, Outcome::Confirmed);
    assert_eq!(
        (r.slot, r.balance_change, r.fee),
        (Some(42), Some(-5000), Some(5000))
    );
    assert!(r.signature.is_some() && r.program_error.is_none());
    assert_eq!(
        r.explorer,
        r.signature.map(|s| format!("https://x.example/tx/{s}"))
    );
}

#[test]
fn outcome_failed_on_chain_reports_the_program_error() {
    let chain = Fake::default();
    chain.statuses.borrow_mut().push_back(Ok(Some(landed(Some(
        serde_json::json!({"InstructionError": [0, {"Custom": 1}]}),
    )))));
    *chain.meta.borrow_mut() = Some(meta(&[
        "Program log: AnchorError: nope",
        "Program X failed: custom program error: 0x1",
    ]));
    let r = confirm_with(&chain);
    assert_eq!(r.outcome, Outcome::Failed);
    assert_eq!(r.outcome.exit_code(), ExitCode::from(12));
    assert_eq!(r.program_error.as_deref(), Some("AnchorError: nope"));
    assert!(r.error.is_some());
}

// Pinchy's re-review, P1-1: behind a load balancer, node A can report a finalized height past
// last_valid while node B, with an older root or incomplete history, finds no status for a
// transaction that landed on A's chain. No pair of answers like that may become `expired`.
#[test]
fn an_expired_blockhash_with_no_status_is_unknown_not_expired() {
    let chain = Fake::default();
    chain.tips.borrow_mut().extend([Ok(10), Ok(101)]);
    chain.history.borrow_mut().push_back(Ok(None));
    let started = std::time::Instant::now();
    let r = confirm_with(&chain);
    assert_eq!(r.outcome, Outcome::Unknown);
    assert_eq!(r.outcome.exit_code(), ExitCode::from(14));
    assert!(
        r.detail
            .is_some_and(|d| d.contains("does not prove it never landed"))
    );
    assert_eq!(r.balance_change, None);
    assert!(
        started.elapsed() < Duration::from_millis(150),
        "past last_valid, waiting cannot help: it ends at once"
    );
}

#[test]
fn outcome_confirmed_when_it_lands_in_the_last_valid_block() {
    let chain = Fake::default();
    chain.tips.borrow_mut().push_back(Ok(101));
    chain.history.borrow_mut().push_back(Ok(Some(landed(None))));
    assert_eq!(confirm_with(&chain).outcome, Outcome::Confirmed);
}

// Pinchy's review, P1: an RPC error on the final status lookup is not absence.
#[test]
fn a_failed_final_lookup_is_not_expiry() {
    let chain = Fake::default();
    chain.steady_tip.set(101);
    for _ in 0..10_000 {
        chain
            .history
            .borrow_mut()
            .push_back(Err(RpcError::Transport("reset".into())));
    }
    let r = confirm_with(&chain);
    assert_eq!(r.outcome, Outcome::Unknown);
    assert!(
        r.detail
            .is_some_and(|d| d.contains("look up the signature"))
    );
}

// Pinchy's review, P1: a processed status can still be dropped; it is not "landed".
#[test]
fn a_processed_status_is_not_landed() {
    let chain = Fake::default();
    chain.steady_tip.set(101);
    for _ in 0..10_000 {
        chain
            .statuses
            .borrow_mut()
            .push_back(Ok(Some(status("processed", None))));
        chain
            .history
            .borrow_mut()
            .push_back(Ok(Some(status("processed", None))));
    }
    assert_eq!(confirm_with(&chain).outcome, Outcome::Unknown);
}

// Pinchy's review, P1: a responsive RPC whose tip never moves must not hold a sign forever.
#[test]
fn a_stalled_chain_ends_at_the_deadline() {
    let chain = Fake::default();
    chain.steady_tip.set(10);
    let started = std::time::Instant::now();
    let r = confirm_with(&chain);
    assert_eq!(r.outcome, Outcome::Unknown);
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// A node for the post-approval rebuild: hands out a fresh blockhash and records exactly
/// which transaction it was asked to simulate.
struct Rebuild {
    hash: Hash,
    sim_err: Option<Value>,
    /// The key's balance before, and as simulated after, the rebuilt transaction.
    lamports: (u64, Option<u64>),
    /// The fee the node quotes for the rebuilt message.
    fee: Option<u64>,
    fee_asked: RefCell<Vec<String>>,
    simulated: RefCell<Vec<String>>,
}

impl Rebuild {
    fn new(sim_err: Option<Value>, lamports: (u64, Option<u64>)) -> Self {
        Self {
            hash: Hash::new_from_array([5; 32]),
            sim_err,
            lamports,
            fee: Some(5_000),
            fee_asked: RefCell::new(vec![]),
            simulated: RefCell::new(vec![]),
        }
    }
}

impl Fresh for Rebuild {
    fn balance(&self, _: &Address) -> R<u64> {
        Ok(self.lamports.0)
    }
    fn fee_for_message(&self, message_b64: &str) -> R<Option<u64>> {
        self.fee_asked.borrow_mut().push(message_b64.to_owned());
        Ok(self.fee)
    }
    fn latest_blockhash(&self) -> R<(Hash, u64)> {
        Ok((self.hash.clone(), 777))
    }
    fn simulate_unsigned(&self, tx_b64: &str, _: &[Address]) -> R<Simulation> {
        self.simulated.borrow_mut().push(tx_b64.to_owned());
        Ok(Simulation {
            err: self.sim_err.clone(),
            logs: vec!["Program log: Error: would fail".into()],
            units: Some(1),
            post_lamports: vec![self.lamports.1],
        })
    }
}

fn approved_payload(payer: &Keypair) -> Payload {
    let text = crate::transfer::fund_payload(
        &payer.pubkey(),
        &Address::new_from_array([2; 32]),
        1,
        "fund",
    )
    .to_string();
    Payload::parse(&text).unwrap()
}

#[test]
fn a_stale_blockhash_is_re_signed_once_then_refused_and_never_sent() {
    let payer = Keypair::new();
    let payload = approved_payload(&payer);
    let stale_tx = || {
        build(
            Message::new(&payload.instructions, Some(&payload.fee_payer)),
            &[&payer],
            Hash::default(),
        )
        .unwrap()
    };
    let node = Rebuild::new(None, (1_000, Some(990)));
    // Fresh: kept as built, nothing simulated again.
    let (_, lv, _) = unless_stale(
        &node,
        || Some(50),
        stale_tx(),
        100,
        &payload,
        &[&payer],
        &payer.pubkey(),
        blank(),
    )
    .unwrap()
    .unwrap();
    assert_eq!((lv, node.simulated.borrow().len()), (100, 0));
    // Unreadable height: unchanged.
    assert!(
        unless_stale(
            &node,
            || None,
            stale_tx(),
            100,
            &payload,
            &[&payer],
            &payer.pubkey(),
            blank()
        )
        .unwrap()
        .is_ok()
    );
    // Stale (height 500 past 100); the fresh one (valid to 777) is used.
    let (tx, lv, _) = unless_stale(
        &node,
        || Some(500),
        stale_tx(),
        100,
        &payload,
        &[&payer],
        &payer.pubkey(),
        blank(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(lv, 777);
    assert_eq!(tx.message.recent_blockhash, node.hash);
    // Stale again after the fresh one: refused, exit 10.
    let r = unless_stale(
        &node,
        || Some(900),
        stale_tx(),
        100,
        &payload,
        &[&payer],
        &payer.pubkey(),
        blank(),
    )
    .unwrap()
    .unwrap_err();
    assert_eq!(
        (r.outcome, r.failed_check),
        (Outcome::Refused, Some("blockhash"))
    );
    assert_eq!(r.outcome.exit_code(), ExitCode::from(10));
}

// Pinchy's review, P1: the post-approval transaction is new bytes and must itself be simulated.
#[test]
fn after_approval_sends_only_the_transaction_it_simulated() {
    let payer = Keypair::new();
    let payload = approved_payload(&payer);
    let node = Rebuild::new(None, (1_000, Some(990)));
    let (tx, last_valid, _) = after_approval(&node, &payload, &[&payer], &payer.pubkey(), blank())
        .unwrap()
        .unwrap();
    assert_eq!(last_valid, 777);
    assert_eq!(
        tx.message.recent_blockhash, node.hash,
        "uses the fresh blockhash"
    );
    assert_eq!(
        node.simulated.borrow().as_slice(),
        [encode_unsigned(&tx).unwrap()],
        "the returned transaction is exactly the one simulated, without its signatures"
    );
}

// Pinchy's re-review, P2: the reservation and the cap check use the rebuilt transaction's own
// simulated spend, not the one shown at approval.
#[test]
fn after_approval_replaces_the_simulated_spend() {
    let payer = Keypair::new();
    let payload = approved_payload(&payer);
    // blank() carries an approval-time spend of -1.
    let spent = Rebuild::new(None, (1_000, Some(400)));
    let (tx, _, r) = after_approval(&spent, &payload, &[&payer], &payer.pubkey(), blank())
        .unwrap()
        .unwrap();
    assert_eq!((r.balance_change, r.compute_units), (Some(-600), Some(1)));
    // Pinchy's re-review of 2f942bf, P2: the fee is quoted for the rebuilt message itself.
    assert_eq!(r.fee, Some(5_000));
    assert_eq!(
        spent.fee_asked.borrow().as_slice(),
        [b64(&tx.message.serialize())]
    );
    // No balance in the new simulation: nothing stale is kept, so `cap_gate` refuses.
    let blind = Rebuild::new(None, (1_000, None));
    let (_, _, r) = after_approval(&blind, &payload, &[&payer], &payer.pubkey(), blank())
        .unwrap()
        .unwrap();
    assert_eq!(r.balance_change, None);
    assert!(cap_gate(&[], &roomy(), &r, 100).is_err());
}

#[test]
fn after_approval_refuses_to_send_when_the_new_simulation_fails() {
    let payer = Keypair::new();
    let payload = approved_payload(&payer);
    let node = Rebuild::new(
        Some(serde_json::json!({"InstructionError": [0, {"Custom": 7}]})),
        (1_000, Some(990)),
    );
    let report = after_approval(&node, &payload, &[&payer], &payer.pubkey(), blank())
        .unwrap()
        .unwrap_err();
    assert_eq!(report.outcome, Outcome::SimulationFailed);
    assert!(
        report
            .detail
            .is_some_and(|d| d.starts_with("after approval"))
    );
    assert_eq!(node.simulated.borrow().len(), 1);
}

fn first_signature(tx: &Transaction) -> String {
    tx.signatures[0].to_string()
}

fn signed_with(payer: &Keypair, payload: &Payload, hash: Hash) -> Transaction {
    build(
        Message::new_with_blockhash(&payload.instructions, Some(&payload.fee_payer), &hash),
        &[payer],
        hash,
    )
    .unwrap()
}

#[test]
fn a_repeated_signature_is_re_signed_once_with_a_fresh_blockhash() {
    let payer = Keypair::new();
    let payload = approved_payload(&payer);
    let node = Rebuild::new(None, (1_000, Some(990)));
    let old = signed_with(&payer, &payload, Hash::default());
    let logged = first_signature(&old);
    let (tx, lv, _) = unless_duplicate(
        &node,
        |s| s == logged,
        old,
        100,
        &payload,
        &[&payer],
        &payer.pubkey(),
        blank(),
    )
    .unwrap()
    .unwrap();
    assert_eq!(tx.message.recent_blockhash, node.hash);
    assert_ne!(first_signature(&tx), logged);
    assert_eq!((lv, node.simulated.borrow().len()), (777, 1));
}

#[test]
fn a_signature_still_repeated_after_a_fresh_blockhash_is_refused() {
    let payer = Keypair::new();
    let payload = approved_payload(&payer);
    let node = Rebuild::new(None, (1_000, Some(990)));
    // The node can only hand out the blockhash the logged transaction used.
    let old = signed_with(&payer, &payload, node.hash.clone());
    let logged = first_signature(&old);
    let r = unless_duplicate(
        &node,
        |s| s == logged,
        old,
        100,
        &payload,
        &[&payer],
        &payer.pubkey(),
        blank(),
    )
    .unwrap()
    .unwrap_err();
    assert_eq!(
        (r.outcome, r.failed_check),
        (Outcome::Refused, Some("blockhash"))
    );
    assert!(r.detail.is_some_and(|d| d.contains("nothing was sent")));
}

// A confirm key's approval rebuild already changed the blockhash: nothing more is done.
#[test]
fn a_new_signature_passes_the_duplicate_guard_untouched() {
    let payer = Keypair::new();
    let payload = approved_payload(&payer);
    let node = Rebuild::new(None, (1_000, Some(990)));
    let logged = first_signature(&signed_with(&payer, &payload, Hash::default()));
    let approved = signed_with(&payer, &payload, node.hash.clone());
    let sig = first_signature(&approved);
    let (tx, lv, _) = unless_duplicate(
        &node,
        |s| s == logged,
        approved,
        100,
        &payload,
        &[&payer],
        &payer.pubkey(),
        blank(),
    )
    .unwrap()
    .unwrap();
    assert_eq!((first_signature(&tx), lv), (sig, 100));
    assert!(node.simulated.borrow().is_empty());
}

#[test]
fn seen_looks_at_this_keys_intents_and_finals_from_the_last_day() {
    let other = Entry {
        key: "other".into(),
        ..intent(100, "b", None)
    };
    let log = [
        intent(100, "a", None),
        sign_final(101, "c", "unknown", None),
        other,
    ];
    assert!(seen(&log, "k", "a", 200));
    assert!(seen(&log, "k", "c", 200));
    assert!(!seen(&log, "k", "b", 200), "another key's entry");
    assert!(!seen(&log, "k", "a", 101 + DAY), "older than 24 h");
    assert!(!seen(&log, "k", "z", 200));
}

fn sign(home: &crate::testrpc::Home) -> Report {
    let mode = Mode::Sign { intent: None };
    execute(&home.cfg, None, &home.payload(1), &mode, None)
        .unwrap()
        .0
}

#[test]
fn the_same_payload_with_the_same_blockhash_is_refused_not_confirmed_twice() {
    let home = crate::testrpc::Home::new("dup-same", "");
    let first = sign(&home);
    assert_eq!(first.outcome, Outcome::Confirmed);
    assert!(first.checks.contains(&"duplicate"));
    let second = sign(&home);
    assert_eq!(
        (second.outcome, second.failed_check),
        (Outcome::Refused, Some("blockhash"))
    );
    assert_eq!(home.node.sends(), 1, "nothing sent the second time");
    let log = Audit::new(&home.cfg.state_dir).entries().unwrap();
    let last = log.last().unwrap();
    assert_eq!(
        (last.command.as_str(), last.outcome.as_str()),
        ("sign", "refused")
    );
    assert_eq!(log.iter().filter(|e| e.command == SIGN_INTENT).count(), 1);
}

#[test]
fn the_same_payload_gets_a_fresh_blockhash_when_the_rpc_has_one() {
    let home = crate::testrpc::Home::new("dup-fresh", "");
    let (a, b) = (Hash::new_from_array([3; 32]), Hash::new_from_array([4; 32]));
    *home.node.blockhashes.lock().unwrap() = [a.clone(), a, b].into();
    let first = sign(&home);
    let second = sign(&home);
    assert_eq!(
        (first.outcome, second.outcome),
        (Outcome::Confirmed, Outcome::Confirmed)
    );
    assert_ne!(first.signature, second.signature);
    assert!(second.checks.contains(&"duplicate"));
    assert_eq!(home.node.sends(), 2);
}

fn check(home: &crate::testrpc::Home) -> Report {
    execute(&home.cfg, None, &home.payload(1), &Mode::Check, None)
        .unwrap()
        .0
}

fn halt_on(home: &crate::testrpc::Home, key: Option<&str>) {
    let h = crate::halt::Halt {
        ts: 1,
        reason: "looks wrong".into(),
        by: "cli".into(),
    };
    crate::halt::set(&home.cfg.state_dir, key, &h).unwrap();
}

#[test]
fn a_halted_key_refuses_to_sign_and_check_previews_it() {
    let home = crate::testrpc::Home::new("halt-sign", "");
    assert_eq!(sign(&home).outcome, Outcome::Confirmed);
    let clean = check(&home);
    assert_eq!(clean.sign_preview.map(|p| p.refusals.len()), Some(0));
    halt_on(&home, Some("k"));
    let r = sign(&home);
    assert_eq!(
        (r.outcome, r.failed_check),
        (Outcome::Refused, Some("halt"))
    );
    assert!(r.detail.is_some_and(|d| d.contains("looks wrong")));
    assert_eq!(home.node.sends(), 1, "nothing sent while halted");
    let c = check(&home);
    assert_eq!(c.outcome, Outcome::Ok, "check still runs");
    let preview = c.sign_preview.unwrap();
    assert_eq!(
        preview.refusals.iter().map(|r| r.check).collect::<Vec<_>>(),
        ["halt"]
    );
    let last = Audit::new(&home.cfg.state_dir).entries().unwrap();
    assert_eq!(last.last().unwrap().outcome, "refused");
}

fn previewed(r: &Report) -> Vec<&'static str> {
    r.sign_preview
        .as_ref()
        .unwrap()
        .refusals
        .iter()
        .map(|r| r.check)
        .collect()
}

// Hardening proposal 1: a clean check that sign then refuses is previewed, and sign agrees.
#[test]
fn check_previews_the_rate_limit_that_sign_then_refuses() {
    let home = crate::testrpc::Home::new("preview-rate", "rate_limit_per_minute = 1\n");
    assert!(previewed(&check(&home)).is_empty());
    assert_eq!(sign(&home).outcome, Outcome::Confirmed);
    let c = check(&home);
    assert_eq!((c.outcome, previewed(&c)), (Outcome::Ok, vec!["limits"]));
    assert_eq!(sign(&home).failed_check, Some("limits"));
}

#[test]
fn check_previews_the_cap_against_its_own_simulation() {
    // The mock simulates a spend of SPEND lamports with no fee.
    let at = crate::testrpc::SPEND;
    let home = crate::testrpc::Home::new("preview-cap-ok", &format!("daily_lamport_cap = {at}\n"));
    assert!(previewed(&check(&home)).is_empty());
    let over = at - 1;
    let home = crate::testrpc::Home::new("preview-cap", &format!("daily_lamport_cap = {over}\n"));
    let c = check(&home);
    assert_eq!((c.outcome, previewed(&c)), (Outcome::Ok, vec!["limits"]));
    let detail = &c.sign_preview.unwrap().refusals[0].detail;
    assert!(detail.contains("for this transaction"), "{detail}");
    let s = sign(&home);
    assert_eq!(s.failed_check, Some("limits"));
    assert_eq!(home.node.sends(), 0);
}

#[test]
fn check_previews_a_deny_key_without_refusing_it() {
    let home = crate::testrpc::Home::new("preview-deny", "approval = \"deny\"\n");
    let c = check(&home);
    assert_eq!((c.outcome, previewed(&c)), (Outcome::Ok, vec!["approval"]));
    assert_eq!(sign(&home).failed_check, Some("approval"));
}

#[test]
fn check_says_a_confirm_key_needs_approval() {
    let home = crate::testrpc::Home::new("preview-confirm", "approval = \"confirm\"\n");
    let p = check(&home).sign_preview.unwrap();
    assert!(p.needs_approval && p.refusals.is_empty());
    let auto = crate::testrpc::Home::new("preview-auto", "");
    assert!(!check(&auto).sign_preview.unwrap().needs_approval);
}

#[test]
fn check_lists_every_sign_only_refusal_in_order() {
    let home = crate::testrpc::Home::new(
        "preview-all",
        "approval = \"deny\"\nrate_limit_per_minute = 0\n",
    );
    halt_on(&home, Some("k"));
    assert_eq!(previewed(&check(&home)), ["approval", "halt", "limits"]);
}

#[test]
fn a_refusal_decided_without_the_rpc_is_made_before_it() {
    let home = crate::testrpc::Home::new("deny-offline", "approval = \"deny\"\n");
    let text = std::fs::read_to_string(&home.path).unwrap();
    let (_, rest) = text.split_once('\n').unwrap();
    std::fs::write(
        &home.path,
        format!("rpc_url = \"http://127.0.0.1:9\"\n{rest}"),
    )
    .unwrap();
    let offline = crate::config::Config::load(&home.path, true).unwrap();
    let mode = Mode::Sign { intent: None };
    let (r, _, _) = execute(&offline, None, &home.payload(1), &mode, None).unwrap();
    assert_eq!(
        r.failed_check,
        Some("approval"),
        "refused before any RPC call"
    );
    let foreign = crate::transfer::fund_payload(&home.pubkey, &home.pubkey, 1, "s");
    let mut other = foreign.clone();
    other["transaction"]["fee_payer"] = Value::from(Address::new_from_array([7; 32]).to_string());
    let (r, _, _) = execute(&offline, Some("k"), &other.to_string(), &Mode::Check, None).unwrap();
    assert_eq!(r.failed_check, Some("fee_payer"));
    assert!(
        execute(&offline, None, &foreign.to_string(), &Mode::Check, None).is_err(),
        "the RPC is needed"
    );
}

#[test]
fn a_halt_on_every_key_refuses_this_key_too() {
    let home = crate::testrpc::Home::new("halt-all", "");
    halt_on(&home, None);
    assert_eq!(sign(&home).failed_check, Some("halt"));
}

#[test]
fn repeated_unknown_outcomes_halt_the_key() {
    let home = crate::testrpc::Home::new("halt-trip", "halt_after_unknown = 2\n");
    let key = home.cfg.key("k").unwrap();
    let audit = Audit::new(&home.cfg.state_dir);
    let report = || Report {
        outcome: Outcome::Unknown,
        ..blank()
    };
    audit.append(sign_final(1, "a", "unknown", None)).unwrap();
    let mut first = report();
    after_unknown(&home.cfg, key, &mut first);
    assert!(
        crate::halt::active(&home.cfg.state_dir, "k")
            .unwrap()
            .is_none()
    );
    audit.append(sign_final(2, "b", "unknown", None)).unwrap();
    let mut second = report();
    after_unknown(&home.cfg, key, &mut second);
    assert!(second.detail.is_some_and(|d| d.contains("halted")));
    assert_eq!(sign(&home).failed_check, Some("halt"));
}

#[test]
fn halt_after_unknown_defaults_to_three() {
    let home = crate::testrpc::Home::new("halt-default", "");
    assert_eq!(home.cfg.key("k").unwrap().halt_after_unknown, 3);
}

#[test]
fn outcome_unknown_after_persistent_rpc_failures_and_rebroadcasts() {
    let chain = Fake::default();
    let err = || RpcError::Transport("down".into());
    for _ in 0..MAX_RPC_FAILURES {
        chain.statuses.borrow_mut().push_back(Err(err()));
        chain.tips.borrow_mut().push_back(Err(err()));
    }
    let r = confirm_with(&chain);
    assert_eq!(r.outcome, Outcome::Unknown);
    assert_eq!(r.outcome.exit_code(), ExitCode::from(14));
    assert!(chain.sends.get() > 1, "rebroadcasts while waiting");
    assert!(r.detail.is_some_and(|d| d.contains("down")));
}

const SAGE: &str = "C4SAgeKLgb3pTLWhVr6NRwWyYFuTR7ZeSXFrzoLwfMzF";
const ACT: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

/// A home whose config names a table listing one SAGE instruction, `act`: the signer at 0,
/// checked against the profile at 1. `extra` goes in the key's block.
fn tabled(tag: &str, extra: &str) -> crate::testrpc::Home {
    use sha2::Digest;
    let mut home = crate::testrpc::Home::new(tag, extra);
    let table = format!(
        r#"{{"format": 1, "cluster": "zink-testnet", "programs": [{{"address": "{SAGE}"}}],
        "instructions": [{{"program": "{SAGE}", "discriminator": "0102030405060708", "name": "act",
          "accounts": {{"min": 2, "max": 2}}, "profiles": [{{"signer": 0, "profile": 1}}]}}]}}"#
    );
    std::fs::write(home.dir.join("table.json"), &table).unwrap();
    let sum = crate::payload::hex(&sha2::Sha256::digest(table.as_bytes()));
    let text = std::fs::read_to_string(&home.path).unwrap();
    std::fs::write(
        &home.path,
        format!(
            "instruction_table = {{ path = \"table.json\", sha256 = \"{sum}\", mode = \"report\" }}\n{text}"
        ),
    )
    .unwrap();
    home.cfg = Config::load(&home.path, true).unwrap();
    home
}

fn act(home: &crate::testrpc::Home, disc: [u8; 8], profile: &Address) -> String {
    let key = home.pubkey.to_string();
    serde_json::json!({
        "version": 1, "summary": "act", "warnings": [],
        "transaction": {
            "instructions": [{
                "program_id": SAGE,
                "accounts": [
                    {"pubkey": key, "is_signer": true, "is_writable": false},
                    {"pubkey": profile.to_string(), "is_signer": false, "is_writable": true},
                ],
                "data": disc.to_vec(),
            }],
            "signers": [key],
            "fee_payer": key,
        },
        "partial_signers": [],
    })
    .to_string()
}

fn found(r: &Report) -> Vec<&'static str> {
    r.findings.iter().map(|f| f.check).collect()
}

#[test]
fn table_findings_are_reported_and_logged_but_never_refuse() {
    let own = Address::new_from_array([5; 32]);
    let home = tabled("table-report", &format!("profile = \"{own}\"\n"));
    let run = |payload: &str, mode: &Mode| execute(&home.cfg, None, payload, mode, None).unwrap().0;
    let sign = Mode::Sign { intent: None };

    let unknown = run(&act(&home, [9; 8], &own), &Mode::Check);
    assert_eq!(
        (unknown.outcome, found(&unknown)),
        (Outcome::Ok, vec!["instructions"])
    );

    let other = Address::new_from_array([6; 32]);
    let elsewhere = run(&act(&home, ACT, &other), &sign);
    assert_eq!(
        (elsewhere.outcome, found(&elsewhere)),
        (Outcome::Confirmed, vec!["profile"]),
        "report mode signs anyway"
    );
    let logged = Audit::new(&home.cfg.state_dir).entries().unwrap();
    let last = logged.last().unwrap();
    assert_eq!(last.findings.len(), 1);
    assert_eq!(last.findings[0].check, "profile");

    let clean = run(&act(&home, ACT, &own), &sign);
    assert_eq!((clean.outcome, found(&clean)), (Outcome::Confirmed, vec![]));
    let json = serde_json::to_string(&clean).unwrap();
    assert!(
        !json.contains("findings"),
        "an empty list is left out: {json}"
    );
    let entries = Audit::new(&home.cfg.state_dir).entries().unwrap();
    assert!(entries.last().unwrap().findings.is_empty());
    assert!(
        Audit::new(&home.cfg.state_dir).verify().unwrap().is_ok(),
        "the chain holds"
    );
}

#[test]
fn a_key_that_requires_approval_signs_only_under_its_approved_policy() {
    use crate::halt::tests::term;
    let home = crate::testrpc::Home::new("policy", "require_policy_approval = true\n");
    let payload = act(&home, ACT, &home.pubkey);
    let run = |cfg: &Config, mode: &Mode| execute(cfg, None, &payload, mode, None).unwrap().0;
    let sign = Mode::Sign { intent: None };

    let refused = run(&home.cfg, &sign);
    assert_eq!(
        (refused.outcome, refused.failed_check),
        (Outcome::Refused, Some("policy"))
    );
    let checked = run(&home.cfg, &Mode::Check);
    assert_eq!(checked.outcome, Outcome::Ok, "check still runs");
    let preview: Vec<&str> = checked
        .sign_preview
        .as_ref()
        .unwrap()
        .refusals
        .iter()
        .map(|r| r.check)
        .collect();
    assert_eq!(preview, ["policy"]);

    let key = home.cfg.key("k").unwrap();
    crate::policy_approval::approve(&home.cfg.state_dir, key, Some(term("yes\n")), 1).unwrap();
    assert_eq!(run(&home.cfg, &sign).outcome, Outcome::Confirmed);
    let entries = Audit::new(&home.cfg.state_dir).entries().unwrap();
    let signs: Vec<&Entry> = entries
        .iter()
        .filter(|e| e.command == "sign" || e.command == SIGN_INTENT)
        .collect();
    assert_eq!(
        signs.len(),
        3,
        "the refused sign, the intent and the final entry"
    );
    assert!(
        signs
            .iter()
            .all(|e| e.policy.as_deref() == Some(key.policy.as_str())),
        "every sign entry records the policy it was made under"
    );

    let text = std::fs::read_to_string(&home.path).unwrap();
    std::fs::write(&home.path, format!("{text}rate_limit_per_minute = 99\n")).unwrap();
    let changed = Config::load(&home.path, true).unwrap();
    assert_ne!(changed.key("k").unwrap().policy, key.policy);
    assert_eq!(
        run(&changed, &sign).failed_check,
        Some("policy"),
        "a changed policy needs approving again"
    );
    assert!(Audit::new(&home.cfg.state_dir).verify().unwrap().is_ok());
}

#[test]
fn the_policy_digest_covers_the_instruction_table() {
    let home = tabled("policy-table", "");
    let key = home.cfg.key("k").unwrap();
    let digest = |t| crate::config::policy_digest(home.cfg.cluster, key, &[], t).unwrap();
    assert_eq!(key.policy, digest(home.cfg.instruction_table.as_ref()));
    assert_ne!(key.policy, digest(None));
}

// A sign reads the log back only to the lookback, and the unknown streak only to a landed sign.
#[test]
fn a_sign_reads_only_the_recent_audit_log() {
    let home = crate::testrpc::Home::new("bounded-reads", "");
    let path = home.cfg.state_dir.join("audit.jsonl");
    std::fs::create_dir_all(&home.cfg.state_dir).unwrap();
    let old = Entry {
        ts: 1,
        key: "k".into(),
        command: "sign".into(),
        outcome: "confirmed".into(),
        ..Entry::default()
    };
    std::fs::write(
        &path,
        format!("not json\n{}\n", serde_json::to_string(&old).unwrap()),
    )
    .unwrap();
    let payload = act(&home, ACT, &home.pubkey);
    let sign = Mode::Sign { intent: None };
    let r = execute(&home.cfg, None, &payload, &sign, None).unwrap().0;
    assert_eq!(r.outcome, Outcome::Confirmed, "{r:?}");
    assert!(halt::trip_after_unknown(&home.cfg.state_dir, "k", 1, now()).is_ok());
}

#[test]
fn a_sign_traces_each_phase_and_no_caller_text() {
    let home = crate::testrpc::Home::new("trace", "");
    let payload = act(&home, ACT, &home.pubkey).replace("\"act\"", "\"SECRET-SUMMARY\"");
    let (r, text) = crate::trace::tests::capture(|| {
        let sign = Mode::Sign {
            intent: Some("SECRET-INTENT".into()),
        };
        execute(&home.cfg, None, &payload, &sign, None).unwrap().0
    });
    assert_eq!(r.outcome, Outcome::Confirmed);
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let closed = |name: &str| {
        lines
            .iter()
            .any(|l| l["span"]["name"] == name && l["fields"]["message"] == "close")
    };
    for name in [
        "sign",
        "key_lock",
        "audit_read",
        "rpc",
        "audit_append",
        "confirm",
    ] {
        assert!(closed(name), "no closed {name} span in\n{text}");
    }
    assert!(
        lines
            .iter()
            .any(|l| l["span"]["method"] == "simulateTransaction"),
        "each RPC call names its method"
    );
    assert!(
        lines.iter().any(|l| l["fields"]["outcome"] == "confirmed"),
        "the outcome is logged"
    );
    for secret in ["SECRET-SUMMARY", "SECRET-INTENT", "127.0.0.1"] {
        assert!(!text.contains(secret), "{secret} leaked into\n{text}");
    }
}

#[test]
fn signs_share_one_rpc_connection_and_read_the_genesis_hash_once() {
    let home = crate::testrpc::Home::new("rpc-reuse", "");
    let sign = Mode::Sign { intent: None };
    for n in 0..2_u8 {
        let mut disc = ACT;
        disc[0] = n;
        let r = execute(
            &home.cfg,
            None,
            &act(&home, disc, &home.pubkey),
            &sign,
            None,
        )
        .unwrap()
        .0;
        assert_eq!(r.outcome, Outcome::Confirmed);
    }
    let methods = home.node.methods.lock().unwrap().clone();
    let genesis = methods.iter().filter(|m| *m == "getGenesisHash").count();
    assert_eq!(genesis, 1, "{methods:?}");
    assert_eq!(
        home.node.peers.lock().unwrap().len(),
        1,
        "one kept-alive connection"
    );
}

#[test]
fn the_first_status_checks_come_sooner_then_settle_at_the_poll() {
    let ms = |n| poll_delay(n, POLL).as_millis();
    assert_eq!(
        (1..=5).map(ms).collect::<Vec<_>>(),
        [400, 800, 1_000, 1_000, 1_000]
    );
    let fast = Duration::from_millis(5);
    assert_eq!(poll_delay(1, fast), fast, "never longer than the poll");
    assert_eq!(poll_delay(u32::MAX, POLL), POLL, "no overflow");
}

#[test]
fn without_a_table_there_are_no_findings() {
    let home = crate::testrpc::Home::new("table-none", "");
    let r = execute(
        &home.cfg,
        None,
        &act(&home, [9; 8], &home.pubkey),
        &Mode::Check,
        None,
    )
    .unwrap()
    .0;
    assert!(r.findings.is_empty());
}
