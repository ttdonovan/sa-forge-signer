# sa-forge-signer-idl-table

Writes [sa-forge-signer](../../README.md)'s instruction table from the Codama IDLs of SAGE, Player Profile and Profile Faction. The table holds only facts about each instruction (its discriminator, accounts, signer and profile slots, where each key index sits) and each IDL's sha256, never the IDL itself, so the IDLs stay where they came from.

```sh
just apps idl-table --cluster zink-testnet --from "sage-forge <commit> hearth/generate-sdks/idls" \
  --out tables/zink-testnet.json <sage.json> <player-profile.json> <profile-faction.json>
```

**Docs:** [IDL table](https://ttdonovan.github.io/sa-forge-signer/apps/idl-table.html) in the book (source: [`docs/src/apps/idl-table.md`](../../docs/src/apps/idl-table.md)).
