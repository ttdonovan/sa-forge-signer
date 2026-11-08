//! Structural checks on the compiled message. None decode game instructions.

use std::fmt;
use std::str::FromStr;

use solana_address::Address;
use solana_message::Message;
use solana_message::compiled_instruction::CompiledInstruction;

use crate::cluster::{
    ASSOCIATED_TOKEN_PROGRAM, COMPUTE_BUDGET_PROGRAM, SYSTEM_PROGRAM, TOKEN_2022_PROGRAM,
    TOKEN_PROGRAM,
};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Refusal {
    pub check: &'static str,
    pub detail: String,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.check, self.detail)
    }
}

pub const fn refuse(check: &'static str, detail: String) -> Refusal {
    Refusal { check, detail }
}

pub struct Policy<'a> {
    pub signer: &'a Address,
    pub allowed_programs: &'a [Address],
    pub transfer_to: &'a [Address],
    pub partial_signers: &'a [Address],
    pub limits: Limits,
}

/// A key's bounds on `ComputeBudget` and System instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_compute_units: u32,
    /// Micro-lamports per compute unit; 0 refuses any priority fee.
    pub max_compute_unit_price: u64,
    /// The sum of all System Transfers in one transaction; `None` is unbounded but the daily cap.
    pub transfer_max_lamports: Option<u64>,
}

impl Limits {
    /// The runtime's maximum compute units, no priority fee, no transfer maximum.
    pub const DEFAULT: Self = Self {
        max_compute_units: 1_400_000,
        max_compute_unit_price: 0,
        transfer_max_lamports: None,
    };
}

// Public System Program instruction index. Only Transfer is allowed at the top level: forge-mcp
// builds no top-level System instruction (new accounts are created inside the game programs), and
// every account-creating form (CreateAccount, CreateAccountWithSeed, Assign, Allocate) can fund or
// take over an account whose keypair the requester holds, moving lamports outside the signer.
const SYS_TRANSFER: u32 = 2;

// Public ComputeBudget instruction tags: SetComputeUnitLimit(u32) and SetComputeUnitPrice(u64).
// The others (the deprecated 0, RequestHeapFrame, SetLoadedAccountsDataSizeLimit) are refused.
const CB_UNIT_LIMIT: u8 = 2;
const CB_UNIT_PRICE: u8 = 3;

// Associated Token Account program instructions that only create an account (rent from the fee
// payer, owned by the token program). RecoverNested (2) moves tokens and is refused.
const ATA_CREATE: u8 = 0;
const ATA_CREATE_IDEMPOTENT: u8 = 1;
// Account positions in both create forms: payer, new account, owner (wallet), mint, system, token.
const ATA_OWNER: usize = 2;
const ATA_SYSTEM: usize = 4;
const ATA_TOKEN_PROGRAM: usize = 5;
const ATA_ACCOUNTS: usize = 6;

/// Every check that needs only the message, by name, in order.
pub fn each(message: &Message, policy: &Policy<'_>) -> [(&'static str, Result<(), Refusal>); 6] {
    [
        ("fee_payer", fee_payer(message, policy)),
        ("programs", programs(message, policy)),
        ("tokens", tokens(message)),
        ("compute_budget", compute_budget(message, policy)),
        ("system", system(message, policy)),
        ("signers", signers(message, policy)),
    ]
}

/// Runs every check that needs only the message; returns the names of the checks passed, or the
/// first refusal.
#[cfg(test)]
pub fn structural(message: &Message, policy: &Policy<'_>) -> Result<Vec<&'static str>, Refusal> {
    let mut passed = Vec::new();
    for (name, result) in each(message, policy) {
        result?;
        passed.push(name);
    }
    Ok(passed)
}

fn fee_payer(message: &Message, policy: &Policy<'_>) -> Result<(), Refusal> {
    match message.account_keys.first() {
        Some(k) if k == policy.signer => Ok(()),
        Some(k) => Err(refuse(
            "fee_payer",
            format!("fee payer {k} is not the signing key {}", policy.signer),
        )),
        None => Err(refuse("fee_payer", "message has no accounts".to_owned())),
    }
}

fn programs(message: &Message, policy: &Policy<'_>) -> Result<(), Refusal> {
    for (i, ix) in message.instructions.iter().enumerate() {
        let program = program_of(message, ix)
            .ok_or_else(|| refuse("programs", format!("instruction {i} has no program id")))?;
        if !policy.allowed_programs.contains(program) {
            return Err(refuse(
                "programs",
                format!("instruction {i} calls {program}, which is not allowed"),
            ));
        }
    }
    Ok(())
}

/// Token programs are never called at the top level, whatever the key's `extra_programs` say: their
/// transfers, approvals and authority changes move assets with no destination rule here, and the
/// daily cap counts lamports only. Token movement in play happens inside SAGE, which checks its own
/// accounts. The ATA program may only create accounts.
fn tokens(message: &Message) -> Result<(), Refusal> {
    let parse = |s: &str| Address::from_str(s).map_err(|e| refuse("tokens", e.to_string()));
    let token = parse(TOKEN_PROGRAM)?;
    let token_2022 = parse(TOKEN_2022_PROGRAM)?;
    let ata = parse(ASSOCIATED_TOKEN_PROGRAM)?;
    for (i, ix) in message.instructions.iter().enumerate() {
        let Some(program) = program_of(message, ix) else {
            continue;
        };
        if program == &token || program == &token_2022 {
            return Err(refuse(
                "tokens",
                format!(
                    "instruction {i} calls token program {program} directly, which is not allowed"
                ),
            ));
        }
        if program == &ata {
            // An empty data field is the legacy Create.
            let tag = ix.data.first().copied().unwrap_or(ATA_CREATE);
            if ix.data.len() > 1 || !matches!(tag, ATA_CREATE | ATA_CREATE_IDEMPOTENT) {
                return Err(refuse(
                    "tokens",
                    format!(
                        "instruction {i} is associated-token instruction {tag}; only create is allowed"
                    ),
                ));
            }
            // The ATA program does not check the token program it is given: it sizes the new account
            // by asking it and makes it the owner, so any other program could take the rent.
            let system = parse(SYSTEM_PROGRAM)?;
            let account = |n: usize| account_of(message, ix, n);
            let known = ix.accounts.len() >= ATA_ACCOUNTS
                && account(ATA_SYSTEM) == Some(&system)
                && account(ATA_TOKEN_PROGRAM).is_some_and(|p| p == &token || p == &token_2022);
            if !known {
                return Err(refuse(
                    "tokens",
                    format!(
                        "instruction {i} creates a token account without the System and SPL Token or Token-2022 programs"
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// The owners of associated token accounts the message creates, other than the signing key and its
/// transfer destinations. Each is a rent recipient no destination rule covers, so the caller must
/// check it on-chain (see `engine::ata_owners`).
pub fn ata_owners(message: &Message, policy: &Policy<'_>) -> Result<Vec<Address>, Refusal> {
    let ata = Address::from_str(ASSOCIATED_TOKEN_PROGRAM)
        .map_err(|e| refuse("ata_owners", e.to_string()))?;
    let mut out: Vec<Address> = Vec::new();
    for (i, ix) in message.instructions.iter().enumerate() {
        if program_of(message, ix) != Some(&ata) {
            continue;
        }
        let owner = (ix.accounts.len() >= ATA_ACCOUNTS)
            .then(|| account_of(message, ix, ATA_OWNER))
            .flatten()
            .ok_or_else(|| {
                refuse(
                    "ata_owners",
                    format!("instruction {i} is missing associated-token accounts"),
                )
            })?;
        if owner != policy.signer && !policy.transfer_to.contains(owner) && !out.contains(owner) {
            out.push(*owner);
        }
    }
    Ok(out)
}

/// `ComputeBudget` passes on program id alone otherwise, so one payload could spend the whole daily
/// cap as a priority fee. Only a bounded unit limit and price, each at most once, no accounts.
fn compute_budget(message: &Message, policy: &Policy<'_>) -> Result<(), Refusal> {
    let cb = Address::from_str(COMPUTE_BUDGET_PROGRAM)
        .map_err(|e| refuse("compute_budget", e.to_string()))?;
    let limits = policy.limits;
    let (mut limit_seen, mut price_seen) = (false, false);
    let no =
        |i: usize, why: String| Err(refuse("compute_budget", format!("instruction {i} {why}")));
    for (i, ix) in message.instructions.iter().enumerate() {
        if program_of(message, ix) != Some(&cb) {
            continue;
        }
        if !ix.accounts.is_empty() {
            return no(i, "is a ComputeBudget instruction with accounts".to_owned());
        }
        let tag = ix.data.first().copied();
        let value = ix.data.get(1..);
        match (tag, value.and_then(|v| <[u8; 4]>::try_from(v).ok())) {
            (Some(CB_UNIT_LIMIT), Some(b)) => {
                let units = u32::from_le_bytes(b);
                if limit_seen {
                    return no(i, "sets the compute unit limit a second time".to_owned());
                }
                if units > limits.max_compute_units {
                    return no(
                        i,
                        format!(
                            "sets a compute unit limit of {units}, over this key's {}",
                            limits.max_compute_units
                        ),
                    );
                }
                limit_seen = true;
                continue;
            }
            (Some(CB_UNIT_LIMIT), None) => {
                return no(i, "is a SetComputeUnitLimit of the wrong length".to_owned());
            }
            _ => {}
        }
        match (tag, value.and_then(|v| <[u8; 8]>::try_from(v).ok())) {
            (Some(CB_UNIT_PRICE), Some(b)) => {
                let price = u64::from_le_bytes(b);
                if price_seen {
                    return no(i, "sets the compute unit price a second time".to_owned());
                }
                if price > limits.max_compute_unit_price {
                    return no(
                        i,
                        format!(
                            "sets a compute unit price of {price} micro-lamports, over this key's {}",
                            limits.max_compute_unit_price
                        ),
                    );
                }
                price_seen = true;
            }
            (Some(CB_UNIT_PRICE), None) => {
                return no(i, "is a SetComputeUnitPrice of the wrong length".to_owned());
            }
            _ => {
                return no(
                    i,
                    format!(
                        "is ComputeBudget instruction {tag:?}; only SetComputeUnitLimit and SetComputeUnitPrice are allowed"
                    ),
                );
            }
        }
    }
    Ok(())
}

fn system(message: &Message, policy: &Policy<'_>) -> Result<(), Refusal> {
    let system = Address::from_str(SYSTEM_PROGRAM).map_err(|e| refuse("system", e.to_string()))?;
    let mut total: u64 = 0;
    for (i, ix) in message.instructions.iter().enumerate() {
        if program_of(message, ix) != Some(&system) {
            continue;
        }
        let tag = ix
            .data
            .get(..4)
            .and_then(|b| <[u8; 4]>::try_from(b).ok())
            .map(u32::from_le_bytes)
            .ok_or_else(|| refuse("system", format!("instruction {i} is too short")))?;
        let account = |n: usize| account_of(message, ix, n);
        if tag == SYS_TRANSFER {
            // Exactly the tag and the amount: anything else is not a Transfer this check can sum.
            let lamports = ix
                .data
                .get(4..)
                .and_then(|b| <[u8; 8]>::try_from(b).ok())
                .map(u64::from_le_bytes)
                .ok_or_else(|| {
                    refuse(
                        "system",
                        format!("instruction {i} is a Transfer of the wrong length"),
                    )
                })?;
            if account(0) != Some(policy.signer) {
                return Err(refuse(
                    "system",
                    format!("instruction {i} transfers from an account other than the signing key"),
                ));
            }
            total = total.saturating_add(lamports);
        }
        match tag {
            SYS_TRANSFER => match account(1) {
                Some(to) if policy.transfer_to.contains(to) => {}
                Some(to) => {
                    return Err(refuse(
                        "system",
                        format!(
                            "instruction {i} transfers to {to}, which is not an allowed destination"
                        ),
                    ));
                }
                None => {
                    return Err(refuse(
                        "system",
                        format!("instruction {i} is missing an account"),
                    ));
                }
            },
            other => {
                return Err(refuse(
                    "system",
                    format!("instruction {i} is System instruction {other}, which is not allowed"),
                ));
            }
        }
    }
    if let Some(max) = policy.limits.transfer_max_lamports
        && total > max
    {
        return Err(refuse(
            "system",
            format!("the transfers total {total} lamports, over this key's {max} per transaction"),
        ));
    }
    Ok(())
}

fn signers(message: &Message, policy: &Policy<'_>) -> Result<(), Refusal> {
    for p in policy.partial_signers {
        if p == policy.signer {
            return Err(refuse(
                "signers",
                "a partial signer is the signing key".to_owned(),
            ));
        }
        let required = message
            .account_keys
            .iter()
            .enumerate()
            .any(|(i, k)| k == p && message.is_signer(i));
        if !required {
            return Err(refuse(
                "signers",
                format!("partial signer {p} is not a signer of the transaction"),
            ));
        }
    }
    for (i, k) in message.account_keys.iter().enumerate() {
        if message.is_signer(i) && k != policy.signer && !policy.partial_signers.contains(k) {
            return Err(refuse(
                "signers",
                format!("{k} must sign, but it is neither the signing key nor a partial signer"),
            ));
        }
    }
    Ok(())
}

fn program_of<'m>(message: &'m Message, ix: &CompiledInstruction) -> Option<&'m Address> {
    message.account_keys.get(usize::from(ix.program_id_index))
}

fn account_of<'m>(message: &'m Message, ix: &CompiledInstruction, n: usize) -> Option<&'m Address> {
    ix.accounts
        .get(n)
        .and_then(|&k| message.account_keys.get(usize::from(k)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_instruction::{AccountMeta, Instruction};

    fn addr(n: u8) -> Address {
        Address::new_from_array([n; 32])
    }

    fn system_ix(tag: u32, accounts: Vec<AccountMeta>) -> Instruction {
        let mut data = tag.to_le_bytes().to_vec();
        data.extend_from_slice(&[0; 8]);
        Instruction {
            program_id: Address::from_str(SYSTEM_PROGRAM).unwrap(),
            accounts,
            data,
        }
    }

    fn run(
        ixs: &[Instruction],
        payer: &Address,
        partial: &[Address],
        to: &[Address],
    ) -> Result<Vec<&'static str>, Refusal> {
        let message = Message::new(ixs, Some(payer));
        let allowed = [Address::from_str(SYSTEM_PROGRAM).unwrap(), addr(9)];
        structural(
            &message,
            &Policy {
                signer: &addr(1),
                allowed_programs: &allowed,
                transfer_to: to,
                partial_signers: partial,
                limits: Limits::DEFAULT,
            },
        )
    }

    fn game_ix() -> Instruction {
        Instruction {
            program_id: addr(9),
            accounts: vec![AccountMeta::new_readonly(addr(1), true)],
            data: vec![1],
        }
    }

    #[test]
    fn passes_plain_game_instruction() {
        assert!(run(&[game_ix()], &addr(1), &[], &[]).is_ok());
    }

    #[test]
    fn refuses_foreign_fee_payer() {
        assert_eq!(
            run(&[game_ix()], &addr(2), &[], &[]).unwrap_err().check,
            "fee_payer"
        );
    }

    #[test]
    fn refuses_unlisted_program() {
        let ix = Instruction {
            program_id: addr(7),
            accounts: vec![],
            data: vec![],
        };
        assert_eq!(
            run(&[ix], &addr(1), &[], &[]).unwrap_err().check,
            "programs"
        );
    }

    #[test]
    fn transfer_only_to_allowed_destinations() {
        let ix = system_ix(
            SYS_TRANSFER,
            vec![
                AccountMeta::new(addr(1), true),
                AccountMeta::new(addr(3), false),
            ],
        );
        assert_eq!(
            run(std::slice::from_ref(&ix), &addr(1), &[], &[])
                .unwrap_err()
                .check,
            "system"
        );
        assert!(run(&[ix], &addr(1), &[], &[addr(3)]).is_ok());
    }

    // Pinchy's review, P0: a requester-held new keypair must not be fundable or assignable at the top
    // level, by any account-creating System instruction, partial signer or not.
    #[test]
    fn refuses_every_account_creating_system_instruction() {
        const CREATE_ACCOUNT: u32 = 0;
        const ASSIGN: u32 = 1;
        const CREATE_WITH_SEED: u32 = 3;
        const ALLOCATE: u32 = 8;
        let payer_and_new = || {
            vec![
                AccountMeta::new(addr(1), true),
                AccountMeta::new(addr(4), true),
            ]
        };
        for (tag, accounts) in [
            (CREATE_ACCOUNT, payer_and_new()),
            (CREATE_WITH_SEED, payer_and_new()),
            (ASSIGN, vec![AccountMeta::new(addr(4), true)]),
            (ALLOCATE, vec![AccountMeta::new(addr(4), true)]),
        ] {
            let ix = system_ix(tag, accounts);
            let refusal = run(&[ix], &addr(1), &[addr(4)], &[]).unwrap_err();
            assert_eq!(
                refusal.check, "system",
                "System instruction {tag} must be refused"
            );
        }
    }

    fn token_ix(program: &str, data: Vec<u8>) -> Instruction {
        Instruction {
            program_id: Address::from_str(program).unwrap(),
            accounts: vec![
                AccountMeta::new(addr(1), true),
                AccountMeta::new(addr(3), false),
            ],
            data,
        }
    }

    fn run_allowing(ixs: &[Instruction], extra: &[&str]) -> Result<Vec<&'static str>, Refusal> {
        let message = Message::new(ixs, Some(&addr(1)));
        let mut allowed = vec![Address::from_str(SYSTEM_PROGRAM).unwrap(), addr(9)];
        allowed.extend(extra.iter().map(|p| Address::from_str(p).unwrap()));
        structural(
            &message,
            &Policy {
                signer: &addr(1),
                allowed_programs: &allowed,
                transfer_to: &[],
                partial_signers: &[],
                limits: Limits::DEFAULT,
            },
        )
    }

    // Pinchy's review, P0: SPL movement had no destination rule. Token programs are refused at the top
    // level even when a key's extra_programs lists them.
    #[test]
    fn refuses_direct_token_program_calls_even_when_listed() {
        const TRANSFER: u8 = 3;
        for program in [TOKEN_PROGRAM, TOKEN_2022_PROGRAM] {
            let ix = token_ix(program, vec![TRANSFER, 1, 0, 0, 0, 0, 0, 0, 0]);
            assert_eq!(
                run_allowing(&[ix], &[program]).unwrap_err().check,
                "tokens",
                "{program} must be refused at the top level"
            );
        }
    }

    #[test]
    fn associated_token_program_may_only_create() {
        const RECOVER_NESTED: u8 = 2;
        for data in [vec![], vec![ATA_CREATE], vec![ATA_CREATE_IDEMPOTENT]] {
            let ix = Instruction {
                data: data.clone(),
                ..ata_create(addr(1))
            };
            assert!(
                run_allowing(&[ix], &[ASSOCIATED_TOKEN_PROGRAM]).is_ok(),
                "ATA create {data:?} is allowed"
            );
        }
        for data in [vec![RECOVER_NESTED], vec![ATA_CREATE_IDEMPOTENT, 0]] {
            let ix = Instruction {
                data: data.clone(),
                ..ata_create(addr(1))
            };
            assert_eq!(
                run_allowing(&[ix], &[ASSOCIATED_TOKEN_PROGRAM])
                    .unwrap_err()
                    .check,
                "tokens",
                "ATA instruction {data:?} must be refused"
            );
        }
    }

    fn ata_create(owner: Address) -> Instruction {
        Instruction {
            program_id: Address::from_str(ASSOCIATED_TOKEN_PROGRAM).unwrap(),
            accounts: vec![
                AccountMeta::new(addr(1), true),
                AccountMeta::new(addr(20), false),
                AccountMeta::new_readonly(owner, false),
                AccountMeta::new_readonly(addr(21), false),
                AccountMeta::new_readonly(Address::from_str(SYSTEM_PROGRAM).unwrap(), false),
                AccountMeta::new_readonly(Address::from_str(TOKEN_PROGRAM).unwrap(), false),
            ],
            data: vec![ATA_CREATE_IDEMPOTENT],
        }
    }

    // An ATA create must name the real System and token programs, or the token program it names
    // owns the new account and its rent.
    #[test]
    fn associated_token_create_needs_the_real_system_and_token_programs() {
        let token_2022 = Instruction {
            accounts: {
                let mut a = ata_create(addr(1)).accounts;
                a[5] = AccountMeta::new_readonly(
                    Address::from_str(TOKEN_2022_PROGRAM).unwrap(),
                    false,
                );
                a
            },
            ..ata_create(addr(1))
        };
        assert!(run_allowing(&[token_2022], &[ASSOCIATED_TOKEN_PROGRAM]).is_ok());
        for (slot, fake) in [(5, addr(9)), (4, addr(9))] {
            let mut ix = ata_create(addr(1));
            ix.accounts[slot] = AccountMeta::new_readonly(fake, false);
            assert_eq!(
                run_allowing(&[ix], &[ASSOCIATED_TOKEN_PROGRAM])
                    .unwrap_err()
                    .check,
                "tokens",
                "account {slot} must be the real program"
            );
        }
        let mut short = ata_create(addr(1));
        short.accounts.truncate(5);
        assert_eq!(
            run_allowing(&[short], &[ASSOCIATED_TOKEN_PROGRAM])
                .unwrap_err()
                .check,
            "tokens"
        );
    }

    fn owners_to_verify(ixs: &[Instruction], to: &[Address]) -> Result<Vec<Address>, Refusal> {
        ata_owners(
            &Message::new(ixs, Some(&addr(1))),
            &Policy {
                signer: &addr(1),
                allowed_programs: &[],
                transfer_to: to,
                partial_signers: &[],
                limits: Limits::DEFAULT,
            },
        )
    }

    // Pinchy's re-review, P1-3: creating a token account pays its rent to whoever owns it. The
    // signing key and its transfer destinations are covered already; any other owner is returned
    // for the on-chain game-account check, once even if repeated.
    #[test]
    fn ata_owners_other_than_the_key_and_its_destinations_need_checking() {
        assert_eq!(owners_to_verify(&[ata_create(addr(1))], &[]), Ok(vec![]));
        assert_eq!(
            owners_to_verify(&[ata_create(addr(3))], &[addr(3)]),
            Ok(vec![])
        );
        assert_eq!(
            owners_to_verify(&[ata_create(addr(7)), ata_create(addr(7)), game_ix()], &[]),
            Ok(vec![addr(7)])
        );
        let mut short = ata_create(addr(7));
        short.accounts.truncate(2);
        assert_eq!(
            owners_to_verify(&[short], &[]).unwrap_err().check,
            "ata_owners"
        );
    }

    #[test]
    fn spl_token_is_not_in_the_default_program_list() {
        assert!(
            crate::cluster::ZINK_TESTNET
                .programs
                .iter()
                .all(|(_, p)| *p != TOKEN_PROGRAM && *p != TOKEN_2022_PROGRAM)
        );
    }

    #[test]
    fn refuses_assigning_the_signing_key() {
        let ix = system_ix(1, vec![AccountMeta::new(addr(1), true)]); // Assign
        assert_eq!(run(&[ix], &addr(1), &[], &[]).unwrap_err().check, "system");
    }

    #[test]
    fn refuses_nonce_and_other_system_instructions() {
        let ix = system_ix(4, vec![AccountMeta::new(addr(5), false)]);
        assert_eq!(run(&[ix], &addr(1), &[], &[]).unwrap_err().check, "system");
    }

    #[test]
    fn refuses_unknown_required_signer_and_stray_partial() {
        let ix = Instruction {
            program_id: addr(9),
            accounts: vec![AccountMeta::new(addr(6), true)],
            data: vec![],
        };
        assert_eq!(run(&[ix], &addr(1), &[], &[]).unwrap_err().check, "signers");
        assert_eq!(
            run(&[game_ix()], &addr(1), &[addr(8)], &[])
                .unwrap_err()
                .check,
            "signers"
        );
        assert_eq!(
            run(&[game_ix()], &addr(1), &[addr(1)], &[])
                .unwrap_err()
                .check,
            "signers"
        );
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use crate::cluster::COMPUTE_BUDGET_PROGRAM;
    use solana_instruction::{AccountMeta, Instruction};

    fn addr(n: u8) -> Address {
        Address::new_from_array([n; 32])
    }

    fn cb(data: Vec<u8>) -> Instruction {
        Instruction {
            program_id: Address::from_str(COMPUTE_BUDGET_PROGRAM).unwrap(),
            accounts: vec![],
            data,
        }
    }

    fn unit_limit(units: u32) -> Instruction {
        let mut d = vec![2];
        d.extend_from_slice(&units.to_le_bytes());
        cb(d)
    }

    fn unit_price(micro_lamports: u64) -> Instruction {
        let mut d = vec![3];
        d.extend_from_slice(&micro_lamports.to_le_bytes());
        cb(d)
    }

    fn transfer(from: Address, to: Address, lamports: u64) -> Instruction {
        let mut data = 2_u32.to_le_bytes().to_vec();
        data.extend_from_slice(&lamports.to_le_bytes());
        Instruction {
            program_id: Address::from_str(SYSTEM_PROGRAM).unwrap(),
            accounts: vec![AccountMeta::new(from, true), AccountMeta::new(to, false)],
            data,
        }
    }

    const LIMITS: Limits = Limits {
        max_compute_units: 1_400_000,
        max_compute_unit_price: 10,
        transfer_max_lamports: Some(100),
    };

    fn run_with(ixs: &[Instruction], limits: Limits, partial: &[Address]) -> Result<(), Refusal> {
        let message = Message::new(ixs, Some(&addr(1)));
        let allowed = [
            Address::from_str(SYSTEM_PROGRAM).unwrap(),
            Address::from_str(COMPUTE_BUDGET_PROGRAM).unwrap(),
        ];
        structural(
            &message,
            &Policy {
                signer: &addr(1),
                allowed_programs: &allowed,
                transfer_to: &[addr(3)],
                partial_signers: partial,
                limits,
            },
        )
        .map(|_| ())
    }

    fn check(ixs: &[Instruction]) -> Option<&'static str> {
        run_with(ixs, LIMITS, &[]).err().map(|r| r.check)
    }

    #[test]
    fn compute_budget_allows_only_a_bounded_unit_limit_and_price() {
        assert_eq!(check(&[unit_limit(1_400_000), unit_price(10)]), None);
        assert_eq!(check(&[unit_limit(1_400_001)]), Some("compute_budget"));
        assert_eq!(check(&[unit_price(11)]), Some("compute_budget"));
        let mut price_zero = LIMITS;
        price_zero.max_compute_unit_price = 0;
        assert_eq!(
            run_with(&[unit_price(1)], price_zero, &[])
                .unwrap_err()
                .check,
            "compute_budget",
            "the default price ceiling refuses any priority fee"
        );
        assert!(run_with(&[unit_price(0)], price_zero, &[]).is_ok());
    }

    #[test]
    fn compute_budget_refuses_other_forms_lengths_accounts_and_repeats() {
        let heap = {
            let mut d = vec![1];
            d.extend_from_slice(&(64 * 1024_u32).to_le_bytes());
            cb(d)
        };
        let loaded = {
            let mut d = vec![4];
            d.extend_from_slice(&1_u32.to_le_bytes());
            cb(d)
        };
        for (what, ix) in [
            ("RequestHeapFrame", heap),
            ("SetLoadedAccountsDataSizeLimit", loaded),
            ("deprecated 0", cb(vec![0, 0, 0, 0, 0, 0, 0, 0, 0])),
            ("empty", cb(vec![])),
            ("limit too short", cb(vec![2, 0, 0, 0])),
            ("limit too long", cb(vec![2, 0, 0, 0, 0, 0])),
            ("price too short", cb(vec![3, 0, 0, 0, 0])),
        ] {
            assert_eq!(check(&[ix]), Some("compute_budget"), "{what}");
        }
        let mut with_account = unit_limit(1);
        with_account.accounts = vec![AccountMeta::new_readonly(addr(5), false)];
        assert_eq!(check(&[with_account]), Some("compute_budget"));
        assert_eq!(
            check(&[unit_limit(1), unit_limit(2)]),
            Some("compute_budget"),
            "two limits in one message"
        );
        assert_eq!(
            check(&[unit_price(1), unit_price(1)]),
            Some("compute_budget")
        );
    }

    #[test]
    fn transfers_are_summed_against_the_per_transaction_maximum() {
        assert_eq!(check(&[transfer(addr(1), addr(3), 100)]), None);
        assert_eq!(
            check(&[
                transfer(addr(1), addr(3), 60),
                transfer(addr(1), addr(3), 41)
            ]),
            Some("system"),
            "summed across instructions"
        );
        let mut unlimited = LIMITS;
        unlimited.transfer_max_lamports = None;
        assert!(run_with(&[transfer(addr(1), addr(3), u64::MAX)], unlimited, &[]).is_ok());
        let mut short = transfer(addr(1), addr(3), 1);
        short.data.truncate(8);
        assert_eq!(check(&[short]), Some("system"));
    }

    #[test]
    fn a_transfer_must_come_from_the_signing_key() {
        let r = run_with(&[transfer(addr(4), addr(3), 1)], LIMITS, &[addr(4)]).unwrap_err();
        assert_eq!(r.check, "system", "{r}");
    }
}
