//! Legacy Solana transaction messages, compiled by hand: the page needs one instruction at a time and
//! no Solana SDK.

pub type Pubkey = [u8; 32];

pub fn parse_pubkey(s: &str) -> Result<Pubkey, String> {
    let bytes = bs58::decode(s.trim())
        .into_vec()
        .map_err(|_| format!("not a base58 address: {s}"))?;
    Pubkey::try_from(bytes.as_slice()).map_err(|_| format!("not a 32-byte address: {s}"))
}

pub fn encode_pubkey(key: &Pubkey) -> String {
    bs58::encode(key).into_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountMeta {
    pub pubkey: Pubkey,
    pub is_signer: bool,
    pub is_writable: bool,
}

impl AccountMeta {
    pub const fn new(pubkey: Pubkey, is_signer: bool, is_writable: bool) -> Self {
        Self {
            pubkey,
            is_signer,
            is_writable,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instruction {
    pub program_id: Pubkey,
    pub accounts: Vec<AccountMeta>,
    pub data: Vec<u8>,
}

pub fn write_compact_u16(out: &mut Vec<u8>, n: usize) -> Result<(), String> {
    let mut v = u16::try_from(n).map_err(|_| format!("{n} does not fit a compact-u16"))?;
    loop {
        let low = u8::try_from(v & 0x7f).map_err(|_| "compact-u16".to_string())?;
        v >>= 7;
        if v == 0 {
            out.push(low);
            return Ok(());
        }
        out.push(low | 0x80);
    }
}

/// Returns the value and the number of bytes it took.
pub fn read_compact_u16(bytes: &[u8]) -> Result<(usize, usize), String> {
    let mut value: usize = 0;
    for (i, byte) in bytes.iter().take(3).enumerate() {
        let shift = u32::try_from(i)
            .ok()
            .and_then(|i| i.checked_mul(7))
            .ok_or("compact-u16")?;
        let part = usize::from(byte & 0x7f)
            .checked_shl(shift)
            .ok_or("compact-u16")?;
        value |= part;
        if byte & 0x80 == 0 {
            let len = i.checked_add(1).ok_or("compact-u16")?;
            return Ok((value, len));
        }
    }
    Err("bad compact-u16".into())
}

/// A legacy message paid for by `fee_payer`.
pub fn compile_message(
    fee_payer: Pubkey,
    instructions: &[Instruction],
    blockhash: Pubkey,
) -> Result<Vec<u8>, String> {
    let mut keys = vec![AccountMeta::new(fee_payer, true, true)];
    let mut add = |meta: &AccountMeta| {
        if let Some(k) = keys.iter_mut().find(|k| k.pubkey == meta.pubkey) {
            k.is_signer |= meta.is_signer;
            k.is_writable |= meta.is_writable;
        } else {
            keys.push(meta.clone());
        }
    };
    for ix in instructions {
        ix.accounts.iter().for_each(&mut add);
    }
    for ix in instructions {
        add(&AccountMeta::new(ix.program_id, false, false));
    }
    // Stable: signer-writable, signer-readonly, writable, readonly; the fee payer stays first.
    keys.sort_by_key(|k| (!k.is_signer, !k.is_writable));

    let count = |f: fn(&AccountMeta) -> bool| u8::try_from(keys.iter().filter(|k| f(k)).count());
    let header = [
        count(|k| k.is_signer),
        count(|k| k.is_signer && !k.is_writable),
        count(|k| !k.is_signer && !k.is_writable),
    ];
    let mut out = Vec::new();
    for h in header {
        out.push(h.map_err(|_| "too many accounts".to_string())?);
    }
    write_compact_u16(&mut out, keys.len())?;
    for k in &keys {
        out.extend_from_slice(&k.pubkey);
    }
    out.extend_from_slice(&blockhash);
    write_compact_u16(&mut out, instructions.len())?;
    let index = |key: &Pubkey| -> Result<u8, String> {
        let i = keys
            .iter()
            .position(|k| k.pubkey == *key)
            .ok_or("account missing")?;
        u8::try_from(i).map_err(|_| "too many accounts".to_string())
    };
    for ix in instructions {
        out.push(index(&ix.program_id)?);
        write_compact_u16(&mut out, ix.accounts.len())?;
        for meta in &ix.accounts {
            out.push(index(&meta.pubkey)?);
        }
        write_compact_u16(&mut out, ix.data.len())?;
        out.extend_from_slice(&ix.data);
    }
    Ok(out)
}

/// The message with empty signature slots, for the wallet to fill.
pub fn unsigned_transaction(message: &[u8]) -> Result<Vec<u8>, String> {
    let signers = usize::from(*message.first().ok_or("empty message")?);
    let mut out = Vec::new();
    write_compact_u16(&mut out, signers)?;
    out.resize(
        out.len()
            .checked_add(signers.checked_mul(64).ok_or("overflow")?)
            .ok_or("overflow")?,
        0,
    );
    out.extend_from_slice(message);
    Ok(out)
}

/// Checks that the wallet signed exactly `message` and filled every signature; returns the first
/// signature, base58, which names the transaction.
pub fn check_signed(signed: &[u8], message: &[u8]) -> Result<String, String> {
    let (count, len) = read_compact_u16(signed)?;
    let end = count
        .checked_mul(64)
        .and_then(|n| n.checked_add(len))
        .ok_or("overflow")?;
    let sigs = signed.get(len..end).ok_or("signed transaction too short")?;
    let rest = signed.get(end..).ok_or("signed transaction too short")?;
    if rest != message {
        return Err(
            "the wallet changed the transaction before signing it (some wallets add their own \
             instructions); nothing was sent"
                .into(),
        );
    }
    if sigs.chunks(64).any(|s| s.iter().all(|b| *b == 0)) || count == 0 {
        return Err("the wallet left a signature empty; nothing was sent".into());
    }
    let first = sigs.get(..64).ok_or("signed transaction too short")?;
    Ok(bs58::encode(first).into_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_u16_round_trips() {
        for (n, bytes) in [
            (0, vec![0]),
            (127, vec![0x7f]),
            (128, vec![0x80, 0x01]),
            (16_383, vec![0xff, 0x7f]),
            (16_384, vec![0x80, 0x80, 0x01]),
            (65_535, vec![0xff, 0xff, 0x03]),
        ] {
            let mut out = Vec::new();
            write_compact_u16(&mut out, n).unwrap();
            assert_eq!(out, bytes);
            assert_eq!(read_compact_u16(&bytes).unwrap(), (n, bytes.len()));
        }
        assert!(write_compact_u16(&mut Vec::new(), 65_536).is_err());
    }

    #[test]
    fn pubkeys_round_trip() {
        let s = "C4PRoFNroxxzdgeCoM31LJjYRg7kT6ymogSTAT99iD1u";
        assert_eq!(encode_pubkey(&parse_pubkey(s).unwrap()), s);
        assert!(parse_pubkey("abc").is_err());
        assert!(parse_pubkey("0OIl").is_err());
    }

    #[test]
    fn compiles_dedups_and_orders_accounts() {
        let (payer, writable, readonly, program) = ([1; 32], [2; 32], [3; 32], [4; 32]);
        let ix = Instruction {
            program_id: program,
            accounts: vec![
                AccountMeta::new(payer, true, true),
                AccountMeta::new(readonly, false, false),
                AccountMeta::new(payer, true, false),
                AccountMeta::new(writable, false, true),
            ],
            data: vec![9, 8],
        };
        let msg = compile_message(payer, &[ix], [7; 32]).unwrap();
        assert_eq!(msg[..4], [1, 0, 2, 4]);
        let keys: Vec<&[u8]> = msg[4..4 + 4 * 32].chunks(32).collect();
        assert_eq!(keys, [&payer[..], &writable, &readonly, &program]);
        assert_eq!(msg[132..164], [7; 32]);
        assert_eq!(msg[164..], [1, 3, 4, 0, 2, 0, 1, 2, 9, 8]);
    }

    #[test]
    fn checks_the_signed_message() {
        let msg = compile_message([1; 32], &[], [7; 32]).unwrap();
        let unsigned = unsigned_transaction(&msg).unwrap();
        assert_eq!(unsigned.len(), 1 + 64 + msg.len());
        assert!(check_signed(&unsigned, &msg).is_err());
        let mut signed = unsigned;
        signed[1] = 5;
        assert_eq!(
            check_signed(&signed, &msg).unwrap(),
            bs58::encode(&signed[1..65]).into_string()
        );
        let mut changed = signed;
        *changed.last_mut().unwrap() ^= 1;
        assert!(check_signed(&changed, &msg).is_err());
    }
}
