//! The instruction table: facts about each instruction read from a Codama IDL (discriminator,
//! accounts, signer and profile slots, where each key index sits), never the IDL itself.
//!
//! Account positions are the IDL's order, which matches the instruction's accounts when every
//! optional account is passed (Codama's default `programId` strategy puts the program id in an
//! omitted slot). `min` and `max` bound the count either way.

use anyhow::{Context, Result, bail};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Bumped when a field changes meaning; the signer refuses a format it does not know.
pub const FORMAT: u32 = 1;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Table {
    pub format: u32,
    pub cluster: String,
    pub generator: String,
    /// Where the IDLs came from, as given (a repository and commit, say).
    pub from: String,
    pub programs: Vec<Program>,
    pub instructions: Vec<Instruction>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Program {
    pub name: String,
    pub address: String,
    pub idl_version: String,
    /// sha256 of the IDL file the table was generated from.
    pub idl_sha256: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Instruction {
    pub program: String,
    /// The 8-byte discriminator, hex.
    pub discriminator: String,
    pub name: String,
    pub accounts: Accounts,
    /// Slots that sign (`isSigner` true or `"either"`).
    pub signers: Vec<usize>,
    pub profiles: Vec<ProfileGroup>,
    /// The slot named `funder`, which pays for what the instruction creates.
    pub funder: Option<usize>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Accounts {
    pub min: usize,
    pub max: usize,
    pub names: Vec<String>,
}

/// A profile check the program makes: `<prefix>Signer` must be a key of `<prefix>Profile`.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ProfileGroup {
    pub prefix: String,
    pub signer: usize,
    pub profile: usize,
    /// The argument holding this key's index in the profile; `None` when the IDL does not
    /// say which one it is.
    pub key_index: Option<KeyIndex>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct KeyIndex {
    pub arg: String,
    /// Byte offset of the `u16` in the instruction data; `None` behind a variable-size argument.
    pub offset: Option<usize>,
}

/// Builds the table from IDL files given as (contents, label for errors).
pub fn build(cluster: &str, from: &str, idls: &[(String, String)]) -> Result<Table> {
    let mut programs = Vec::new();
    let mut instructions = Vec::new();
    for (text, label) in idls {
        let root: Value = serde_json::from_str(text).with_context(|| format!("parsing {label}"))?;
        // A Codama file is a rootNode holding the program, or the programNode itself.
        let program = root.get("program").unwrap_or(&root);
        if program.get("kind").and_then(Value::as_str) != Some("programNode") {
            bail!("{label}: not a Codama programNode");
        }
        let address =
            str_of(program, "publicKey").with_context(|| format!("{label}: publicKey"))?;
        let types = program.get("definedTypes").and_then(Value::as_array);
        let mut seen = Vec::new();
        for ix in program
            .get("instructions")
            .and_then(Value::as_array)
            .with_context(|| format!("{label}: instructions"))?
        {
            let built = instruction(ix, address, types.map_or(&[][..], Vec::as_slice))
                .with_context(|| format!("{label}: {}", str_of(ix, "name").unwrap_or("?")))?;
            if seen.contains(&built.discriminator) {
                bail!(
                    "{label}: discriminator {} appears twice",
                    built.discriminator
                );
            }
            seen.push(built.discriminator.clone());
            instructions.push(built);
        }
        programs.push(Program {
            name: str_of(program, "name").unwrap_or_default().to_owned(),
            address: address.to_owned(),
            idl_version: str_of(program, "version").unwrap_or_default().to_owned(),
            idl_sha256: hex(&Sha256::digest(text.as_bytes())),
        });
    }
    programs.sort_by(|a, b| a.address.cmp(&b.address));
    instructions.sort_by(|a, b| (&a.program, &a.name).cmp(&(&b.program, &b.name)));
    Ok(Table {
        format: FORMAT,
        cluster: cluster.to_owned(),
        generator: format!("sa-forge-signer-idl-table {}", env!("CARGO_PKG_VERSION")),
        from: from.to_owned(),
        programs,
        instructions,
    })
}

fn instruction(ix: &Value, program: &str, types: &[Value]) -> Result<Instruction> {
    let name = str_of(ix, "name").context("name")?.to_owned();
    let accounts = ix
        .get("accounts")
        .and_then(Value::as_array)
        .context("accounts")?;
    let names: Vec<String> = accounts
        .iter()
        .map(|a| str_of(a, "name").map(str::to_owned).context("account name"))
        .collect::<Result<_>>()?;
    let optional = accounts
        .iter()
        .filter(|a| a.get("isOptional").and_then(Value::as_bool) == Some(true))
        .count();
    let signers = accounts
        .iter()
        .enumerate()
        .filter(|(_, a)| match a.get("isSigner") {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => s == "either",
            _ => false,
        })
        .map(|(i, _)| i)
        .collect();
    let args = ix
        .get("arguments")
        .and_then(Value::as_array)
        .context("arguments")?;
    let discriminator = discriminator(args)?;
    let offsets = offsets(args, types);
    Ok(Instruction {
        program: program.to_owned(),
        discriminator,
        accounts: Accounts {
            min: names.len().saturating_sub(optional),
            max: names.len(),
            names: names.clone(),
        },
        signers,
        profiles: groups(&names, &offsets),
        funder: names.iter().position(|n| n == "funder"),
        name,
    })
}

/// The first argument must be the 8-byte discriminator with its value.
fn discriminator(args: &[Value]) -> Result<String> {
    let first = args.first().context("no arguments")?;
    if str_of(first, "name") != Some("discriminator") {
        bail!("the first argument is not the discriminator");
    }
    let value = first.get("defaultValue").context("discriminator value")?;
    if str_of(value, "encoding") != Some("base16") {
        bail!("discriminator is not base16");
    }
    let data = str_of(value, "data").context("discriminator data")?;
    if data.len() != 16 || !data.chars().all(|c| c.is_ascii_hexdigit()) {
        bail!("discriminator {data:?} is not 8 bytes");
    }
    Ok(data.to_ascii_lowercase())
}

/// Each argument's name and byte offset, `None` once a variable-size argument precedes it.
fn offsets(args: &[Value], types: &[Value]) -> Vec<(String, Option<usize>)> {
    let mut at = Some(0_usize);
    let mut out = Vec::new();
    for a in args {
        let name = str_of(a, "name").unwrap_or_default().to_owned();
        out.push((name, at));
        at = at.and_then(|o| o.checked_add(size(a.get("type")?, types, 0)?));
    }
    out
}

/// A type's fixed size in bytes, or `None` if it varies (strings, vectors, options, data enums).
fn size(t: &Value, types: &[Value], depth: u8) -> Option<usize> {
    if depth > 16 {
        return None;
    }
    let next = depth.saturating_add(1);
    match str_of(t, "kind")? {
        "numberTypeNode" => match str_of(t, "format")? {
            "u8" | "i8" => Some(1),
            "u16" | "i16" => Some(2),
            "u32" | "i32" | "f32" => Some(4),
            "u64" | "i64" | "f64" => Some(8),
            "u128" | "i128" => Some(16),
            _ => None,
        },
        "booleanTypeNode" => size_or(t.get("size"), types, next, 1),
        "publicKeyTypeNode" => Some(32),
        "fixedSizeTypeNode" => t
            .get("size")
            .and_then(Value::as_u64)
            .and_then(|s| usize::try_from(s).ok()),
        "amountTypeNode" | "dateTimeTypeNode" | "solAmountTypeNode" => {
            size(t.get("number")?, types, next)
        }
        "structTypeNode" => t
            .get("fields")?
            .as_array()?
            .iter()
            .try_fold(0_usize, |acc, f| {
                acc.checked_add(size(f.get("type")?, types, next)?)
            }),
        "tupleTypeNode" => t
            .get("items")?
            .as_array()?
            .iter()
            .try_fold(0_usize, |acc, i| acc.checked_add(size(i, types, next)?)),
        "arrayTypeNode" => {
            let count = t.get("count")?;
            if str_of(count, "kind")? != "fixedCountNode" {
                return None;
            }
            let n = usize::try_from(count.get("value")?.as_u64()?).ok()?;
            size(t.get("item")?, types, next)?.checked_mul(n)
        }
        "enumTypeNode" => {
            let scalar = t
                .get("variants")?
                .as_array()?
                .iter()
                .all(|v| str_of(v, "kind") == Some("enumEmptyVariantTypeNode"));
            if scalar {
                size_or(t.get("size"), types, next, 1)
            } else {
                None
            }
        }
        "definedTypeLinkNode" => {
            let name = str_of(t, "name")?;
            let def = types.iter().find(|d| str_of(d, "name") == Some(name))?;
            size(def.get("type")?, types, next)
        }
        _ => None,
    }
}

fn size_or(t: Option<&Value>, types: &[Value], depth: u8, default: usize) -> Option<usize> {
    t.map_or(Some(default), |t| size(t, types, depth))
}

/// `<prefix>Signer` and `<prefix>Profile` pairs, each with the key-index argument for it: named
/// `<prefix>KeyIndex` (a trailing `Validation` dropped), else `keyIndex` for the `profile` and
/// `profileValidation` groups, else the one argument and the one group left over. Then Player
/// Profile's `key` and `profile`, with the empty prefix.
fn groups(names: &[String], args: &[(String, Option<usize>)]) -> Vec<ProfileGroup> {
    let key_args: Vec<&(String, Option<usize>)> = args
        .iter()
        .filter(|(n, _)| n == "keyIndex" || n.ends_with("KeyIndex"))
        .collect();
    let mut out: Vec<ProfileGroup> = Vec::new();
    for (signer, n) in names.iter().enumerate() {
        let Some(prefix) = n.strip_suffix("Signer") else {
            continue;
        };
        let Some(profile) = names
            .iter()
            .position(|p| p.strip_suffix("Profile") == Some(prefix))
        else {
            continue;
        };
        let base = prefix.strip_suffix("Validation").unwrap_or(prefix);
        let by_name = key_args
            .iter()
            .find(|(a, _)| a.strip_suffix("KeyIndex") == Some(base))
            .or_else(|| {
                matches!(base, "profile" | "")
                    .then(|| key_args.iter().find(|(a, _)| a == "keyIndex"))
                    .flatten()
            });
        out.push(ProfileGroup {
            prefix: prefix.to_owned(),
            signer,
            profile,
            key_index: by_name.map(|(arg, offset)| KeyIndex {
                arg: arg.clone(),
                offset: *offset,
            }),
        });
    }
    // One group and one argument left unmatched: they belong together.
    let used: Vec<String> = out
        .iter()
        .filter_map(|g| g.key_index.as_ref().map(|k| k.arg.clone()))
        .collect();
    let free: Vec<&&(String, Option<usize>)> =
        key_args.iter().filter(|(a, _)| !used.contains(a)).collect();
    let open: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, g)| g.key_index.is_none())
        .map(|(i, _)| i)
        .collect();
    if let ([(arg, offset)], [i]) = (free.as_slice(), open.as_slice())
        && let Some(g) = out.get_mut(*i)
    {
        g.key_index = Some(KeyIndex {
            arg: arg.clone(),
            offset: *offset,
        });
    }
    // Player Profile names its own pair `key` and `profile`; it takes `keyIndex` only if unused.
    if let (Some(signer), Some(profile)) = (
        names.iter().position(|n| n == "key"),
        names.iter().position(|n| n == "profile"),
    ) {
        let taken = |a: &str| {
            out.iter()
                .any(|g| g.key_index.as_ref().is_some_and(|k| k.arg == a))
        };
        let key_index = key_args
            .iter()
            .find(|(a, _)| a == "keyIndex" && !taken(a))
            .map(|(arg, offset)| KeyIndex {
                arg: arg.clone(),
                offset: *offset,
            });
        out.push(ProfileGroup {
            prefix: String::new(),
            signer,
            profile,
            key_index,
        });
    }
    out
}

fn str_of<'v>(v: &'v Value, key: &str) -> Option<&'v str> {
    v.get(key).and_then(Value::as_str)
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Table {
        let text = include_str!("../tests/fixtures/game.json").to_owned();
        build("zink-testnet", "fixture", &[(text, "game.json".to_owned())]).unwrap()
    }

    fn ix<'t>(t: &'t Table, name: &str) -> &'t Instruction {
        t.instructions.iter().find(|i| i.name == name).unwrap()
    }

    #[test]
    fn records_the_source_but_not_the_idl() {
        let t = fixture();
        assert_eq!(
            (t.format, t.cluster.as_str(), t.from.as_str()),
            (1, "zink-testnet", "fixture")
        );
        let p = &t.programs[0];
        assert_eq!(
            (p.name.as_str(), p.address.as_str(), p.idl_version.as_str()),
            (
                "gameStarFrame",
                "C4SAgeKLgb3pTLWhVr6NRwWyYFuTR7ZeSXFrzoLwfMzF",
                "1.2.3"
            )
        );
        assert_eq!(p.idl_sha256.len(), 64);
        let names: Vec<&str> = t.instructions.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "addKeys",
                "adminRename",
                "moveFleet",
                "nameThing",
                "operatorAct"
            ],
            "sorted"
        );
    }

    #[test]
    fn a_player_instruction_binds_its_signer_profile_and_key_index() {
        let t = fixture();
        let m = ix(&t, "moveFleet");
        assert_eq!(m.discriminator, "0102030405060708");
        assert_eq!((m.accounts.min, m.accounts.max), (6, 6));
        assert_eq!(m.signers, [0, 5]);
        assert_eq!(m.funder, Some(5));
        assert_eq!(
            m.profiles,
            [ProfileGroup {
                prefix: "profileValidation".into(),
                signer: 0,
                profile: 1,
                key_index: Some(KeyIndex {
                    arg: "keyIndex".into(),
                    offset: Some(8)
                }),
            }]
        );
    }

    #[test]
    fn a_player_profile_key_and_profile_form_a_group() {
        let t = fixture();
        assert_eq!(
            ix(&t, "addKeys").profiles,
            [ProfileGroup {
                prefix: String::new(),
                signer: 1,
                profile: 2,
                key_index: Some(KeyIndex {
                    arg: "keyIndex".into(),
                    offset: Some(8)
                }),
            }]
        );
    }

    #[test]
    fn key_indexes_pair_by_name_and_offsets_cross_fixed_structs() {
        let t = fixture();
        let a = ix(&t, "adminRename");
        assert_eq!(
            (a.accounts.min, a.accounts.max),
            (4, 5),
            "one optional account"
        );
        assert_eq!(a.signers, [0, 2], "\"either\" counts as a signer");
        let k: Vec<(&str, &str, Option<usize>)> = a
            .profiles
            .iter()
            .map(|g| {
                let k = g.key_index.as_ref().unwrap();
                (g.prefix.as_str(), k.arg.as_str(), k.offset)
            })
            .collect();
        // 8 (discriminator) + 5 (the `pair` struct: u32 and u8).
        assert_eq!(
            k,
            [
                ("admin", "adminKeyIndex", Some(13)),
                ("current", "currentKeyIndex", Some(15))
            ]
        );
    }

    #[test]
    fn a_leftover_group_and_argument_pair_up() {
        let t = fixture();
        let o = ix(&t, "operatorAct");
        let k: Vec<(&str, Option<&str>)> = o
            .profiles
            .iter()
            .map(|g| {
                (
                    g.prefix.as_str(),
                    g.key_index.as_ref().map(|k| k.arg.as_str()),
                )
            })
            .collect();
        assert_eq!(
            k,
            [
                ("profileValidation", Some("operatorKeyIndex")),
                ("admin", Some("adminKeyIndex"))
            ]
        );
    }

    #[test]
    fn a_key_index_behind_a_variable_argument_has_no_offset() {
        let t = fixture();
        let k = ix(&t, "nameThing").profiles[0].key_index.as_ref().unwrap();
        assert_eq!((k.arg.as_str(), k.offset), ("keyIndex", None));
    }

    #[test]
    fn refuses_what_is_not_a_codama_program() {
        let bad = |text: &str| build("c", "f", &[(text.to_owned(), "x".to_owned())]).is_err();
        assert!(bad("{"));
        assert!(bad(r#"{"kind": "rootNode", "program": {"kind": "other"}}"#));
        let text = include_str!("../tests/fixtures/game.json");
        assert!(
            bad(&text.replace("0102030405060708", "0102")),
            "short discriminator"
        );
        assert!(
            bad(&text.replace("1112131415161718", "0102030405060708")),
            "repeated discriminator"
        );
        // A rootNode wrapping the program is read the same.
        let wrapped = format!(r#"{{"kind": "rootNode", "program": {text}}}"#);
        assert!(!bad(&wrapped));
    }
}
