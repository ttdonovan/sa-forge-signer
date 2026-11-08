---
description: "sa-forge-signer-idl-table: generates the signer's instruction table (tables/<cluster>.json) from the SAGE, Player Profile and Profile Faction Codama IDLs without copying them; what the table holds, what it cannot tell, and regenerating it after a program upgrade"
---

# IDL table

`sa-forge-signer-idl-table` turns the programs' Codama IDLs into the instruction table that [hardening proposal 4](../dev/hardening.md#4-instruction-table-and-profile-binding) checks payloads against. It writes only facts about each instruction, plus each IDL's sha256 and where the IDLs came from, so the IDLs are never checked in here. The IDLs of these programs are not published on Z.ink testnet (no Anchor IDL account, and no Program Metadata program); today they come from a sage-forge checkout.

```sh
just apps idl-table --cluster zink-testnet --from "sage-forge <commit> hearth/generate-sdks/idls" \
  --out tables/zink-testnet.json <sage.json> <player-profile.json> <profile-faction.json>
```

It refuses a file that is not a Codama `programNode` (or a `rootNode` holding one), an instruction whose first argument is not its 8-byte discriminator, and a discriminator used twice in one program.

## What the table holds

The output is JSON, sorted, so a regeneration diffs cleanly. `format` (1 today) changes when a field changes meaning. `programs` lists each program's name, address, IDL version and the IDL file's sha256. Each entry of `instructions` has:

| Field | Meaning |
|---|---|
| `program`, `discriminator`, `name` | The program address, the 8-byte discriminator in hex, and the instruction's name |
| `accounts` | `min` and `max` account counts (they differ by the optional accounts) and the account `names` in IDL order |
| `signers` | Account positions that sign (`isSigner` true or `"either"`) |
| `profiles` | Each profile check the program makes: a `<prefix>Signer` account that must be a key of the `<prefix>Profile` account, with `key_index`: the argument holding that key's index and its byte `offset` in the instruction data |
| `funder` | The position of the account named `funder`, which pays for what the instruction creates |

Player Profile's `key` signer and `profile` account form a group with the empty prefix. A key-index argument is paired with its group by name (`currentKeyIndex` with `current`, `gameAdminKeyIndex` with `gameAdminValidation`, `keyIndex` with `profile` or `profileValidation`); a single group and argument left unmatched are paired with each other.

## What it cannot tell

- **Offsets behind variable-size arguments.** A key index after a string, vector, option or data-carrying enum has no fixed `offset` (`null`).
- **Key indexes the IDL does not name.** `addKeys` takes `keyAddIndex` and `keyPermissionsIndex`, and SAGE's `addShipEscrow` has a `key` pair beside its `profileSigner` group that uses `keyIndex`; those groups have no `key_index` (`null`).
- **Player Profile's role instructions** (`createRole`, `setRoleName`, ...) take a key index but no `key` signer, so they have no `profiles` entry.
- **Families** (movement, cargo, recovery, ...) are not in the IDL at all.
- **Positions when an optional account is left out.** Positions follow the IDL's order, which holds when every optional account is passed (Codama's default puts the program id in an omitted slot).

## After a program upgrade

Regenerate from the new IDLs with `--from` naming their source, and commit the table in the same change as anything that depends on it. The `programs` hashes say which IDLs a table came from.

`just apps idl-table-check` is its gate: fmt, clippy, tests, advisories and dependency age. Like every app it stays out of the signer's build.
