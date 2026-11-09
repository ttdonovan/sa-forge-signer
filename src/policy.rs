//! Every check `check` and `sign` make, as one pure function: `evaluate` reads no RPC, file or
//! clock. What it needs from outside is gathered first into `Facts` (see `facts`), so the two
//! paths cannot drift and each check is tested without a node.

use solana_address::Address;
use solana_message::Message;

use crate::checks::{self, Refusal, refuse};
use crate::cluster::Cluster;
use crate::config::{Approval, KeyConfig};
use crate::halt::Halt;
use crate::instruction_table::InstructionTable;

/// What is to be signed. `message` may carry any blockhash: no check depends on it.
pub struct Request<'a> {
    pub fee_payer: &'a Address,
    pub message: &'a Message,
    pub partial_signers: &'a [Address],
}

/// The key's configured policy, resolved.
pub struct KeyPolicy<'a> {
    pub key: &'a KeyConfig,
    pub cluster: &'a Cluster,
    pub allowed_programs: &'a [Address],
    pub game_programs: &'a [Address],
    pub table: Option<&'a InstructionTable>,
}

impl KeyPolicy<'_> {
    pub const fn structural<'a>(&'a self, request: &'a Request<'a>) -> checks::Policy<'a> {
        checks::Policy {
            signer: &self.key.pubkey,
            allowed_programs: self.allowed_programs,
            transfer_to: self.key.transfer_to.as_slice(),
            partial_signers: request.partial_signers,
            limits: self.key.limits,
        }
    }
}

/// This key's signs in the last minute and lamports spent or reserved in the last 24 h.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub signs_last_minute: usize,
    pub spent: u64,
}

/// Everything read from outside. A `None` field has not been read yet, and the checks that need
/// it are undecided.
#[derive(Debug, Default)]
pub struct Facts {
    pub halt: Option<Halt>,
    /// The key's approved policy digest, read only for a key that requires approval.
    pub approved_policy: Option<String>,
    pub usage: Usage,
    pub genesis_hash: Option<String>,
    pub partial_signers_exist: Option<Vec<bool>>,
    /// Each owner of an associated token account the message creates (other than the key and its
    /// destinations), and the program that owns it on-chain (`None`: no such account).
    pub ata_owner_programs: Option<Vec<(Address, Option<Address>)>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// `check`: the sign-only checks are evaluated but never refuse.
    Check,
    BeforeSimulation,
    /// With the reservation of the exact transaction about to be sent: its simulated spend plus
    /// its fee, `None` when either is unknown.
    AfterSimulation {
        reserve: Option<u64>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Status {
    Pass,
    Fail(Refusal),
    /// Its facts are not read yet.
    Undecided,
}

impl From<Result<(), Refusal>> for Status {
    fn from(r: Result<(), Refusal>) -> Self {
        r.map_or_else(Self::Fail, |()| Self::Pass)
    }
}

#[derive(Debug)]
struct Item {
    /// Its name in the report's `checks` when it passes; `None` passes silently.
    listed: Option<&'static str>,
    sign_only: bool,
    status: Status,
}

/// Every check's result, in order.
#[derive(Debug)]
pub struct Verdict {
    stage: Stage,
    items: Vec<Item>,
    /// The instruction table's findings, in report mode: they never refuse.
    pub findings: Vec<Refusal>,
}

impl Verdict {
    /// The checks that refuse at this stage.
    fn refusing(&self) -> impl Iterator<Item = &Item> {
        self.items
            .iter()
            .filter(|i| !(self.stage == Stage::Check && i.sign_only))
    }

    /// The refusal, once it is decided: the first failure, with every check before it passed.
    pub fn refusal(&self) -> Option<&Refusal> {
        for i in self.refusing() {
            match &i.status {
                Status::Pass => {}
                Status::Fail(r) => return Some(r),
                Status::Undecided => return None,
            }
        }
        None
    }

    /// The listed checks that passed, up to the first that did not.
    pub fn passed(&self) -> Vec<&'static str> {
        self.refusing()
            .take_while(|i| i.status == Status::Pass)
            .filter_map(|i| i.listed)
            .collect()
    }

    /// Every failed check, in order, whether or not it refuses at this stage.
    #[cfg(test)]
    pub fn failures(&self) -> Vec<&Refusal> {
        self.items
            .iter()
            .filter_map(|i| match &i.status {
                Status::Fail(r) => Some(r),
                _ => None,
            })
            .collect()
    }

    /// The sign-only checks that fail: what `sign` would refuse that `check` does not.
    pub fn sign_only_failures(&self) -> Vec<Refusal> {
        self.items
            .iter()
            .filter(|i| i.sign_only)
            .filter_map(|i| match &i.status {
                Status::Fail(r) => Some(r.clone()),
                _ => None,
            })
            .collect()
    }
}

pub fn evaluate(
    request: &Request<'_>,
    facts: &Facts,
    policy: &KeyPolicy<'_>,
    stage: Stage,
) -> Verdict {
    let key = policy.key;
    let structural = policy.structural(request);
    let mut items = Vec::new();
    let mut add = |listed, sign_only, status| {
        items.push(Item {
            listed,
            sign_only,
            status,
        });
    };
    add(None, false, fee_payer(request, key).into());
    add(None, true, approval(key).into());
    add(
        None,
        true,
        facts
            .halt
            .as_ref()
            .map_or(Ok(()), |h| Err(h.refusal(&key.name)))
            .into(),
    );
    add(
        None,
        true,
        crate::policy_approval::check(key, facts.approved_policy.as_deref()).into(),
    );
    add(Some("chain"), false, chain(facts, policy.cluster));
    for (name, result) in checks::each(request.message, &structural) {
        add(Some(name), false, result.into());
    }
    add(Some("new_accounts"), false, new_accounts(request, facts));
    add(
        Some("ata_owners"),
        false,
        ata_owners(request, &structural, facts, policy),
    );
    add(Some("limits"), true, limits(facts.usage, key).into());
    if let Stage::AfterSimulation { reserve } = stage {
        add(Some("cap"), true, cap(facts.usage, key, reserve).into());
    }
    let findings = policy.table.map_or_else(Vec::new, |t| {
        t.findings(request.message, &key.pubkey, key.profile.as_ref())
    });
    Verdict {
        stage,
        items,
        findings,
    }
}

fn fee_payer(request: &Request<'_>, key: &KeyConfig) -> Result<(), Refusal> {
    if request.fee_payer == &key.pubkey {
        return Ok(());
    }
    Err(refuse(
        "fee_payer",
        format!(
            "payload fee payer {} is not key {:?} ({})",
            request.fee_payer, key.name, key.pubkey
        ),
    ))
}

fn approval(key: &KeyConfig) -> Result<(), Refusal> {
    if key.approval == Approval::Deny {
        return Err(refuse(
            "approval",
            format!("key {:?} is set to deny", key.name),
        ));
    }
    Ok(())
}

fn chain(facts: &Facts, cluster: &Cluster) -> Status {
    match &facts.genesis_hash {
        None => Status::Undecided,
        Some(g) if g == cluster.genesis_hash => Status::Pass,
        Some(g) => Status::Fail(refuse(
            "chain",
            format!("RPC genesis hash {g} is not {}", cluster.name),
        )),
    }
}

fn new_accounts(request: &Request<'_>, facts: &Facts) -> Status {
    let Some(exist) = &facts.partial_signers_exist else {
        return Status::Undecided;
    };
    exist.iter().position(|e| *e).map_or(Status::Pass, |i| {
        let who = request
            .partial_signers
            .get(i)
            .map(ToString::to_string)
            .unwrap_or_default();
        Status::Fail(refuse(
            "new_accounts",
            format!("partial signer {who} already exists on-chain"),
        ))
    })
}

/// An associated token account created for an owner other than the signing key or its transfer
/// destinations must belong to a game account: an account owned by one of the cluster's built-in
/// game programs (a character, a cargo cache), never one only a key's `extra_programs` owns. Its
/// rent then stays where only game logic can move it, rather than in a wallet the requester could
/// close the account from and reclaim the rent.
fn ata_owners(
    request: &Request<'_>,
    structural: &checks::Policy<'_>,
    facts: &Facts,
    policy: &KeyPolicy<'_>,
) -> Status {
    let owners = match checks::ata_owners(request.message, structural) {
        Ok(o) => o,
        Err(r) => return Status::Fail(r),
    };
    if owners.is_empty() {
        return Status::Pass;
    }
    let Some(read) = &facts.ata_owner_programs else {
        return Status::Undecided;
    };
    for who in &owners {
        let program = read.iter().find(|(o, _)| o == who).and_then(|(_, p)| *p);
        if !program.is_some_and(|p| policy.game_programs.contains(&p)) {
            return Status::Fail(refuse(
                "ata_owners",
                format!(
                    "a token account would be created for {who}, which is neither this key, one of its transfer destinations, nor an account of a game program"
                ),
            ));
        }
    }
    Status::Pass
}

/// The rate limit, and the daily cap as spent or reserved so far.
pub fn limits(usage: Usage, key: &KeyConfig) -> Result<(), Refusal> {
    let recent = usage.signs_last_minute;
    if recent >= usize::try_from(key.rate_limit_per_minute).unwrap_or(usize::MAX) {
        return Err(refuse(
            "limits",
            format!("rate limit: {recent} signs in the last minute"),
        ));
    }
    let (spent, cap) = (usage.spent, key.daily_lamport_cap);
    if spent >= cap {
        return Err(refuse(
            "limits",
            format!("daily cap: {spent} of {cap} lamports spent or reserved in 24 h"),
        ));
    }
    Ok(())
}

pub fn unbounded() -> Refusal {
    refuse(
        "limits",
        "the simulation did not report this key's balance, or the RPC gave no fee estimate, so the spend cannot be bounded"
            .to_owned(),
    )
}

/// The cap against the transaction about to be sent: what is spent or reserved plus its own
/// reservation must stay within it.
pub fn cap(usage: Usage, key: &KeyConfig, reserve: Option<u64>) -> Result<(), Refusal> {
    let reserve = reserve.ok_or_else(unbounded)?;
    let (spent, cap) = (usage.spent, key.daily_lamport_cap);
    if spent.saturating_add(reserve) > cap {
        return Err(refuse(
            "limits",
            format!(
                "daily cap: {spent} lamports spent or reserved in 24 h, plus {reserve} for this transaction, exceeds {cap}"
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::{ASSOCIATED_TOKEN_PROGRAM, SYSTEM_PROGRAM, TOKEN_PROGRAM, ZINK_TESTNET};
    use crate::config::KeyClass;
    use solana_instruction::{AccountMeta, Instruction};
    use std::str::FromStr;

    fn addr(n: u8) -> Address {
        Address::new_from_array([n; 32])
    }

    fn program(p: &str) -> Address {
        Address::from_str(p).unwrap()
    }

    /// A payload, key and facts that pass every check; each row changes one thing.
    struct Case {
        key: KeyConfig,
        fee_payer: Address,
        instructions: Vec<Instruction>,
        partial: Vec<Address>,
        facts: Facts,
    }

    fn game_ix() -> Instruction {
        Instruction {
            program_id: addr(9),
            accounts: vec![AccountMeta::new(addr(1), true)],
            data: vec![1],
        }
    }

    fn ata_create(owner: Address) -> Instruction {
        Instruction {
            program_id: program(ASSOCIATED_TOKEN_PROGRAM),
            accounts: vec![
                AccountMeta::new(addr(1), true),
                AccountMeta::new(addr(20), false),
                AccountMeta::new_readonly(owner, false),
                AccountMeta::new_readonly(addr(21), false),
                AccountMeta::new_readonly(program(SYSTEM_PROGRAM), false),
                AccountMeta::new_readonly(program(TOKEN_PROGRAM), false),
            ],
            data: vec![1],
        }
    }

    fn passing() -> Case {
        Case {
            key: KeyConfig {
                name: "k".into(),
                class: KeyClass::Session,
                allow_unattended: false,
                pubkey: addr(1),
                profile: None,
                approval: Approval::Auto,
                transfer_to: vec![addr(3)],
                extra_programs: vec![],
                rate_limit_per_minute: 2,
                daily_lamport_cap: 1_000,
                halt_after_unknown: 3,
                limits: checks::Limits::DEFAULT,
                require_policy_approval: false,
                policy: "p".into(),
            },
            fee_payer: addr(1),
            instructions: vec![game_ix(), ata_create(addr(7))],
            partial: vec![],
            facts: Facts {
                halt: None,
                approved_policy: None,
                usage: Usage {
                    signs_last_minute: 1,
                    spent: 500,
                },
                genesis_hash: Some(ZINK_TESTNET.genesis_hash.to_owned()),
                partial_signers_exist: Some(vec![]),
                ata_owner_programs: Some(vec![(addr(7), Some(addr(9)))]),
            },
        }
    }

    fn run(case: &Case, stage: Stage) -> Verdict {
        let message = Message::new(&case.instructions, Some(&case.fee_payer));
        let allowed = [
            program(SYSTEM_PROGRAM),
            program(crate::cluster::COMPUTE_BUDGET_PROGRAM),
            program(ASSOCIATED_TOKEN_PROGRAM),
            program(TOKEN_PROGRAM),
            addr(9),
        ];
        let request = Request {
            fee_payer: &case.fee_payer,
            message: &message,
            partial_signers: &case.partial,
        };
        let policy = KeyPolicy {
            key: &case.key,
            cluster: &ZINK_TESTNET,
            allowed_programs: &allowed,
            game_programs: &[addr(9)],
            table: None,
        };
        evaluate(&request, &case.facts, &policy, stage)
    }

    fn names(refusals: &[&Refusal]) -> Vec<&'static str> {
        refusals.iter().map(|r| r.check).collect()
    }

    const BEFORE: Stage = Stage::BeforeSimulation;

    #[test]
    fn the_passing_case_passes_every_stage() {
        let c = passing();
        let sign = run(&c, BEFORE);
        assert_eq!(sign.refusal(), None);
        assert_eq!(
            sign.passed(),
            [
                "chain",
                "fee_payer",
                "programs",
                "tokens",
                "compute_budget",
                "system",
                "signers",
                "new_accounts",
                "ata_owners",
                "limits"
            ]
        );
        assert_eq!(run(&c, Stage::Check).passed().last(), Some(&"ata_owners"));
        let after = run(&c, Stage::AfterSimulation { reserve: Some(500) });
        assert_eq!(
            (after.refusal(), after.passed().last()),
            (None, Some(&"cap"))
        );
    }

    /// One row per check: what to change, the stage, and the check that must refuse.
    #[test]
    #[expect(
        clippy::too_many_lines,
        reason = "one row per check reads better as one table"
    )]
    fn each_check_refuses() {
        type Change = fn(&mut Case);
        let after = |reserve| Stage::AfterSimulation { reserve };
        let rows: Vec<(&str, Change, Stage, &str)> = vec![
            (
                "payload fee payer",
                |c| c.fee_payer = addr(2),
                BEFORE,
                "fee_payer",
            ),
            (
                "deny key",
                |c| c.key.approval = Approval::Deny,
                BEFORE,
                "approval",
            ),
            (
                "halted",
                |c| {
                    c.facts.halt = Some(Halt {
                        ts: 1,
                        reason: "stop".into(),
                        by: "cli".into(),
                    });
                },
                BEFORE,
                "halt",
            ),
            (
                "policy not approved",
                |c| c.key.require_policy_approval = true,
                BEFORE,
                "policy",
            ),
            (
                "policy changed since approval",
                |c| {
                    c.key.require_policy_approval = true;
                    c.facts.approved_policy = Some("old".into());
                },
                BEFORE,
                "policy",
            ),
            (
                "wrong chain",
                |c| c.facts.genesis_hash = Some("x".into()),
                BEFORE,
                "chain",
            ),
            (
                "unlisted program",
                |c| {
                    c.instructions.push(Instruction {
                        program_id: addr(8),
                        accounts: vec![],
                        data: vec![],
                    });
                },
                BEFORE,
                "programs",
            ),
            (
                "direct token call",
                |c| {
                    c.instructions.push(Instruction {
                        program_id: program(TOKEN_PROGRAM),
                        accounts: vec![AccountMeta::new(addr(1), true)],
                        data: vec![3],
                    });
                },
                BEFORE,
                "tokens",
            ),
            (
                "transfer to a stranger",
                |c| {
                    let mut data = 2_u32.to_le_bytes().to_vec();
                    data.extend_from_slice(&[0; 8]);
                    c.instructions.push(Instruction {
                        program_id: program(SYSTEM_PROGRAM),
                        accounts: vec![
                            AccountMeta::new(addr(1), true),
                            AccountMeta::new(addr(4), false),
                        ],
                        data,
                    });
                },
                BEFORE,
                "system",
            ),
            (
                "unknown signer",
                |c| {
                    c.instructions.push(Instruction {
                        program_id: addr(9),
                        accounts: vec![AccountMeta::new(addr(6), true)],
                        data: vec![],
                    });
                },
                BEFORE,
                "signers",
            ),
            (
                "partial signer exists",
                |c| {
                    c.partial = vec![addr(5)];
                    c.instructions.push(Instruction {
                        program_id: addr(9),
                        accounts: vec![AccountMeta::new(addr(5), true)],
                        data: vec![],
                    });
                    c.facts.partial_signers_exist = Some(vec![true]);
                },
                BEFORE,
                "new_accounts",
            ),
            (
                "token account for a non-game owner",
                |c| c.facts.ata_owner_programs = Some(vec![(addr(7), Some(addr(8)))]),
                BEFORE,
                "ata_owners",
            ),
            (
                "token account for a missing owner",
                |c| c.facts.ata_owner_programs = Some(vec![(addr(7), None)]),
                BEFORE,
                "ata_owners",
            ),
            (
                "malformed token account create",
                |c| c.instructions[1].accounts.truncate(5),
                BEFORE,
                "tokens",
            ),
            (
                "priority fee",
                |c| {
                    c.instructions.push(Instruction {
                        program_id: program(crate::cluster::COMPUTE_BUDGET_PROGRAM),
                        accounts: vec![],
                        data: vec![3, 1, 0, 0, 0, 0, 0, 0, 0],
                    });
                },
                BEFORE,
                "compute_budget",
            ),
            (
                "transfers over the maximum",
                |c| {
                    c.key.limits.transfer_max_lamports = Some(9);
                    let mut data = 2_u32.to_le_bytes().to_vec();
                    data.extend_from_slice(&10_u64.to_le_bytes());
                    c.instructions.push(Instruction {
                        program_id: program(SYSTEM_PROGRAM),
                        accounts: vec![
                            AccountMeta::new(addr(1), true),
                            AccountMeta::new(addr(3), false),
                        ],
                        data,
                    });
                },
                BEFORE,
                "system",
            ),
            (
                "rate limit",
                |c| c.facts.usage.signs_last_minute = 2,
                BEFORE,
                "limits",
            ),
            (
                "cap spent",
                |c| c.facts.usage.spent = 1_000,
                BEFORE,
                "limits",
            ),
            ("cap unbounded", |_| {}, after(None), "limits"),
            (
                "cap with this transaction",
                |_| {},
                after(Some(501)),
                "limits",
            ),
        ];
        for (what, change, stage, check) in rows {
            let mut c = passing();
            change(&mut c);
            let v = run(&c, stage);
            assert_eq!(
                v.refusal().map(|r| r.check),
                Some(check),
                "{what}: {:?}",
                v.failures()
            );
        }
        assert_eq!(
            run(&passing(), after(Some(500))).refusal(),
            None,
            "exactly the cap"
        );
    }

    #[test]
    fn every_failed_check_is_returned_in_order() {
        let mut c = passing();
        c.key.approval = Approval::Deny;
        c.facts.usage.signs_last_minute = 9;
        c.instructions.push(Instruction {
            program_id: addr(8),
            accounts: vec![],
            data: vec![],
        });
        c.facts.genesis_hash = Some("x".into());
        assert_eq!(
            names(&run(&c, BEFORE).failures()),
            ["approval", "chain", "programs", "limits"]
        );
    }

    // `check` and `sign` evaluate the same checks on the same facts; only which refuse differs.
    #[test]
    fn check_and_sign_give_the_same_verdict_on_the_same_facts() {
        let mut gates_only = passing();
        gates_only.key.approval = Approval::Deny;
        gates_only.facts.usage.spent = 1_000;
        let mut both = passing();
        both.facts.usage.signs_last_minute = 9;
        both.facts.partial_signers_exist = Some(vec![true]);
        both.partial = vec![addr(5)];
        for c in [passing(), gates_only, both] {
            let (check, sign) = (run(&c, Stage::Check), run(&c, BEFORE));
            assert_eq!(names(&check.failures()), names(&sign.failures()));
            assert_eq!(check.sign_only_failures(), sign.sign_only_failures());
            // Every fact is read, so `sign` refuses on the first failure, and `check` on the first
            // that is not sign-only.
            assert_eq!(sign.refusal(), check.failures().first().copied());
            let preview = check.sign_only_failures();
            assert_eq!(
                check.refusal(),
                check.failures().into_iter().find(|r| !preview.contains(r))
            );
        }
        let mut c = passing();
        c.key.approval = Approval::Deny;
        c.facts.usage.spent = 1_000;
        assert_eq!(
            run(&c, Stage::Check).refusal(),
            None,
            "check never refuses a sign-only gate"
        );
        assert_eq!(run(&c, BEFORE).refusal().map(|r| r.check), Some("approval"));
        assert_eq!(
            names(
                &run(&c, Stage::Check)
                    .sign_only_failures()
                    .iter()
                    .collect::<Vec<_>>()
            ),
            ["approval", "limits"]
        );
    }

    // A refusal is decided only once every check before it has passed, so the engine makes it
    // before reading the next RPC fact.
    #[test]
    fn a_refusal_waits_for_the_checks_before_it() {
        let mut c = passing();
        c.facts.genesis_hash = None;
        c.facts.partial_signers_exist = None;
        c.facts.ata_owner_programs = None;
        assert_eq!(run(&c, BEFORE).refusal(), None);
        assert!(run(&c, BEFORE).passed().is_empty());
        c.facts.usage.signs_last_minute = 9;
        assert_eq!(
            run(&c, BEFORE).refusal(),
            None,
            "limits waits for the chain"
        );
        c.key.approval = Approval::Deny;
        assert_eq!(run(&c, BEFORE).refusal().map(|r| r.check), Some("approval"));
        c.key.approval = Approval::Auto;
        c.fee_payer = addr(2);
        assert_eq!(
            run(&c, Stage::Check).refusal().map(|r| r.check),
            Some("fee_payer")
        );
    }
}
