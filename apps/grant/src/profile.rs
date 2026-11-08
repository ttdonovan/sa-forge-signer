//! The Player Profile program's `Profile` account, `AddKeys` and `RemoveKeys`, written by hand from
//! its IDL (its generated SDK is not public). `tests/` checks them against bytes from that SDK.

use crate::tx::{AccountMeta, Instruction, Pubkey, parse_pubkey};

pub const PLAYER_PROFILE_PROGRAM: &str = "C4PRoFNroxxzdgeCoM31LJjYRg7kT6ymogSTAT99iD1u";
pub const SAGE_PROGRAM: &str = "C4SAgeKLgb3pTLWhVr6NRwWyYFuTR7ZeSXFrzoLwfMzF";
pub const PROFILE_FACTION_PROGRAM: &str = "C4FACQA1PpNRKrjQ2862ABNR42DTz7EzGj1uhTNFASwP";
pub const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";

pub const PROFILE_DISCRIMINATOR: [u8; 8] = [0xb8, 0x65, 0xa5, 0xbc, 0x5f, 0x3f, 0x7f, 0xbc];
const ADD_KEYS_DISCRIMINATOR: [u8; 8] = [0x9a, 0xe1, 0x10, 0xb0, 0x92, 0x4c, 0x4e, 0x69];
const REMOVE_KEYS_DISCRIMINATOR: [u8; 8] = [0xac, 0x41, 0x64, 0x3f, 0x1f, 0xd0, 0x39, 0x47];

/// Discriminator 8, version 1, auth key count 2, threshold 1, next seq id 8, created at 8, key count 2.
pub const FIRST_KEY_OFFSET: usize = 30;
/// key 32, scope 32, expire time 8, permissions 8.
pub const KEY_LEN: usize = 80;

/// `ProfilePermissions` bits that matter here.
pub const AUTH: u64 = 1;
pub const ADD_KEYS: u64 = 1 << 1;
pub const REMOVE_KEYS: u64 = 1 << 2;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileKey {
    pub key: Pubkey,
    pub scope: Pubkey,
    /// Unix seconds; -1 never expires.
    pub expire_time: i64,
    pub permissions: u64,
}

impl ProfileKey {
    pub const fn expired(&self, now: i64) -> bool {
        self.expire_time >= 0 && now >= self.expire_time
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub auth_key_count: u16,
    pub key_threshold: u8,
    pub keys: Vec<ProfileKey>,
}

struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let (head, rest) = self
            .0
            .split_first_chunk::<N>()
            .ok_or_else(|| "the profile account is shorter than its layout".to_string())?;
        self.0 = rest;
        Ok(*head)
    }
}

pub fn decode_profile(data: &[u8]) -> Result<Profile, String> {
    let mut r = Reader(data);
    if r.take::<8>()? != PROFILE_DISCRIMINATOR {
        return Err("not a Player Profile account".into());
    }
    let _version = r.take::<1>()?;
    let auth_key_count = u16::from_le_bytes(r.take()?);
    let [key_threshold] = r.take::<1>()?;
    let _next_seq_id = r.take::<8>()?;
    let _created_at = r.take::<8>()?;
    let count = u16::from_le_bytes(r.take()?);
    let keys = (0..count)
        .map(|_| {
            Ok(ProfileKey {
                key: r.take()?,
                scope: r.take()?,
                expire_time: i64::from_le_bytes(r.take()?),
                permissions: u64::from_le_bytes(r.take()?),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(Profile {
        auth_key_count,
        key_threshold,
        keys,
    })
}

/// The index of `wallet`'s unexpired profile-scoped key holding `needed` (or `AUTH`).
pub fn authority_index(
    profile: &Profile,
    wallet: &Pubkey,
    needed: u64,
    now: i64,
) -> Result<u16, String> {
    if profile.key_threshold > 1 {
        return Err(format!(
            "the profile needs {} auth signatures; this page signs with one wallet only",
            profile.key_threshold
        ));
    }
    let scope = parse_pubkey(PLAYER_PROFILE_PROGRAM)?;
    let index = profile
        .keys
        .iter()
        .position(|k| {
            k.key == *wallet
                && k.scope == scope
                && !k.expired(now)
                && k.permissions & (needed | AUTH) != 0
        })
        .ok_or_else(|| {
            "the connected wallet is not a key on this profile with the permission to do this"
                .to_string()
        })?;
    u16::try_from(index).map_err(|_| "key index out of range".to_string())
}

pub struct Grant {
    pub new_key: Pubkey,
    pub scope: Pubkey,
    pub expire_time: i64,
    pub permissions: u64,
}

/// `AddKeys` adding one key, authorised by the wallet's key at `auth_index`, which also pays.
pub fn add_keys(
    wallet: Pubkey,
    profile: Pubkey,
    auth_index: u16,
    grant: &Grant,
) -> Result<Instruction, String> {
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(&ADD_KEYS_DISCRIMINATOR);
    data.extend_from_slice(&auth_index.to_le_bytes()); // key_add_index
    data.extend_from_slice(&auth_index.to_le_bytes()); // key_permissions_index
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&grant.scope);
    data.extend_from_slice(&grant.expire_time.to_le_bytes());
    data.extend_from_slice(&grant.permissions.to_le_bytes());
    Ok(Instruction {
        program_id: parse_pubkey(PLAYER_PROFILE_PROGRAM)?,
        accounts: vec![
            AccountMeta::new(wallet, true, true),
            AccountMeta::new(wallet, true, false),
            AccountMeta::new(profile, false, true),
            AccountMeta::new(parse_pubkey(SYSTEM_PROGRAM)?, false, false),
            AccountMeta::new(grant.new_key, false, false),
        ],
        data,
    })
}

/// `RemoveKeys` removing the single key at `index`.
pub fn remove_keys(
    wallet: Pubkey,
    profile: Pubkey,
    auth_index: u16,
    index: u16,
) -> Result<Instruction, String> {
    let end = index
        .checked_add(1)
        .ok_or_else(|| "key index out of range".to_string())?;
    let mut data = Vec::with_capacity(14);
    data.extend_from_slice(&REMOVE_KEYS_DISCRIMINATOR);
    data.extend_from_slice(&auth_index.to_le_bytes());
    data.extend_from_slice(&index.to_le_bytes());
    data.extend_from_slice(&end.to_le_bytes());
    Ok(Instruction {
        program_id: parse_pubkey(PLAYER_PROFILE_PROGRAM)?,
        accounts: vec![
            AccountMeta::new(wallet, true, true),
            AccountMeta::new(wallet, true, false),
            AccountMeta::new(profile, false, true),
            AccountMeta::new(parse_pubkey(SYSTEM_PROGRAM)?, false, false),
        ],
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> Pubkey {
        [byte; 32]
    }

    fn encode(profile: &Profile) -> Vec<u8> {
        let mut out = PROFILE_DISCRIMINATOR.to_vec();
        out.push(1);
        out.extend_from_slice(&profile.auth_key_count.to_le_bytes());
        out.push(profile.key_threshold);
        out.extend_from_slice(&[0; 16]);
        out.extend_from_slice(&u16::try_from(profile.keys.len()).unwrap().to_le_bytes());
        for k in &profile.keys {
            out.extend_from_slice(&k.key);
            out.extend_from_slice(&k.scope);
            out.extend_from_slice(&k.expire_time.to_le_bytes());
            out.extend_from_slice(&k.permissions.to_le_bytes());
        }
        out
    }

    fn sample() -> Profile {
        let profile_scope = parse_pubkey(PLAYER_PROFILE_PROGRAM).unwrap();
        Profile {
            auth_key_count: 1,
            key_threshold: 1,
            keys: vec![
                ProfileKey {
                    key: key(1),
                    scope: profile_scope,
                    expire_time: -1,
                    permissions: u64::MAX,
                },
                ProfileKey {
                    key: key(2),
                    scope: parse_pubkey(SAGE_PROGRAM).unwrap(),
                    expire_time: 1_000,
                    permissions: 1 << 10,
                },
            ],
        }
    }

    #[test]
    fn decodes_what_it_encodes_with_keys_at_the_memcmp_offset() {
        let bytes = encode(&sample());
        assert_eq!(bytes.len(), FIRST_KEY_OFFSET + 2 * KEY_LEN);
        assert_eq!(bytes[FIRST_KEY_OFFSET..FIRST_KEY_OFFSET + 32], key(1));
        assert_eq!(decode_profile(&bytes).unwrap(), sample());
    }

    #[test]
    fn refuses_short_or_foreign_accounts() {
        let bytes = encode(&sample());
        assert!(decode_profile(&bytes[..bytes.len() - 1]).is_err());
        let mut foreign = bytes;
        foreign[0] ^= 1;
        assert!(decode_profile(&foreign).is_err());
    }

    #[test]
    fn authority_needs_an_unexpired_profile_scoped_key_with_the_permission() {
        let p = sample();
        assert_eq!(authority_index(&p, &key(1), ADD_KEYS, 0).unwrap(), 0);
        // A SAGE-scoped session key cannot grant.
        assert!(authority_index(&p, &key(2), ADD_KEYS, 0).is_err());
        assert!(authority_index(&p, &key(9), ADD_KEYS, 0).is_err());
        let mut multi = p;
        multi.key_threshold = 2;
        assert!(authority_index(&multi, &key(1), ADD_KEYS, 0).is_err());
    }

    #[test]
    fn expiry() {
        let k = &sample().keys[1];
        assert!(!k.expired(999));
        assert!(k.expired(1_000));
        assert!(!sample().keys[0].expired(i64::MAX));
    }

    #[test]
    fn remove_keys_takes_a_one_key_range() {
        let ix = remove_keys(key(1), key(3), 0, 4).unwrap();
        assert_eq!(&ix.data[8..], &[0, 0, 4, 0, 5, 0]);
        assert!(remove_keys(key(1), key(3), 0, u16::MAX).is_err());
    }

    /// One instruction of a forge-tx `SerializableTransaction` fixture.
    fn fixture(json: &str) -> Instruction {
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        let ix = &v["instructions"][0];
        let pk = |s: &serde_json::Value| parse_pubkey(s.as_str().unwrap()).unwrap();
        Instruction {
            program_id: pk(&ix["program_id"]),
            accounts: ix["accounts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| {
                    AccountMeta::new(
                        pk(&a["pubkey"]),
                        a["is_signer"].as_bool().unwrap(),
                        a["is_writable"].as_bool().unwrap(),
                    )
                })
                .collect(),
            data: serde_json::from_value(ix["data"].clone()).unwrap(),
        }
    }

    // Fixtures: forge-tx's build_add_profile_key_transaction and build_remove_profile_key_transaction
    // (Player Profile SDK) with funder [1; 32], new key [2; 32], profile [3; 32], authority index 0.
    #[test]
    fn add_keys_matches_the_sdk() {
        let grant = Grant {
            new_key: key(2),
            scope: parse_pubkey(SAGE_PROGRAM).unwrap(),
            expire_time: 1_800_000_000,
            permissions: (1 << 63) | (1 << 10),
        };
        let ix = add_keys(key(1), key(3), 0, &grant).unwrap();
        assert_eq!(ix, fixture(include_str!("../tests/fixtures/add_keys.json")));
    }

    // Fixture: solana-sdk's Message::new_with_blockhash for add_keys.json, blockhash [7; 32].
    #[test]
    fn add_keys_message_matches_solana_sdk() {
        let ix = fixture(include_str!("../tests/fixtures/add_keys.json"));
        let message = crate::tx::compile_message(key(1), &[ix], [7; 32]).unwrap();
        let expected: Vec<u8> =
            serde_json::from_str(include_str!("../tests/fixtures/add_keys_message.json")).unwrap();
        assert_eq!(message, expected);
    }

    #[test]
    fn remove_keys_matches_the_sdk() {
        let ix = remove_keys(key(1), key(3), 0, 4).unwrap();
        assert_eq!(
            ix,
            fixture(include_str!("../tests/fixtures/remove_keys.json"))
        );
    }
}
