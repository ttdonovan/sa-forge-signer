---
description: "sa-forge-signer-grant: a local web page where the profile authority's own browser wallet (or a Ledger through it) grants and revokes a SAGE session key, for Path B. What it checks, refuses and leaves out. Early spike."
---

# Grant page

`sa-forge-signer-grant` is a web page, served on `127.0.0.1`, for the human who holds the profile wallet ([Path B](../setup.md#path-b-the-human-holds-the-wallet)). It does the grant and revoke steps of the [key lifecycle](../key-lifecycle.md): it builds a Player Profile `AddKeys` or `RemoveKeys` from what you enter, simulates it, and has your browser wallet sign it. It holds no keys and never sees one. It is an early spike in `apps/grant/`: a Leptos app compiled to WebAssembly, with one small JavaScript file for the wallet and RPC calls and no npm dependencies.

Open it in the browser where your wallet extension already runs. Wallet extensions do not run inside desktop app webviews, so this is a page, not a desktop app.

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
just apps grant          # http://127.0.0.1:8791
```

Trunk downloads the `wasm-bindgen` CLI matching `apps/grant/Cargo.lock` the first time it builds, unless that version is already installed.

## Granting a session key

1. **Network:** the RPC defaults to `https://rpc1.z.ink`. Every step that uses it (connecting, reading the profile, reviewing, signing) first checks its genesis hash and refuses any network other than Z.ink testnet. Editing the RPC field clears the check and any transaction under review, and a transaction reviewed against one URL is not sent through another.
2. **Wallet:** wallets that support Wallet Standard and `solana:signTransaction` appear as buttons. Connecting looks for profiles that hold the wallet among their first four keys, and loads the profile if there is exactly one. Otherwise paste its address.
3. **Profile:** the table shows every key: index, pubkey, scope, permission names and expiry.
4. **Grant:** paste the session key's pubkey (`sa-forge-signer key show <name>`), choose permissions (presets or individual SAGE player actions), and an expiration: 7, 30, 60 or 90 days, or No expiration (`expire_time` -1, the key works until it is revoked). It defaults to 7 days, and the page shows the date it ends. **Review** re-reads the profile, builds the instruction and simulates it.
5. **Sign:** check the review, then **Sign with wallet and send**. The log links the transaction on the explorer, and once it confirms shows the new key's index. Give that index to the agent, which passes it as forge-mcp's `keyIndex`; the signer config has no index field, and its `profile` setting is informational.

> **Caution.** The permission bit numbers and names the page offers are not from a primary source. They are transcribed from SAGE forge's permission tables, which are not public, because the programs' IDLs do not define them. Only `ADD_KEYS`, `AUTH` and SAGE's `DOCK` (bit 12) have been confirmed on-chain so far. A wrong bit grants a different action than its name says, so check a new grant against what the agent can and cannot do before relying on it.

The bits, as `apps/grant/src/perms.rs` defines them (`b(bit, name, player)`, where `player` marks a bit the page offers to a session key):

```rust
{{#include ../../../apps/grant/src/perms.rs:bits}}
```

The presets, by SAGE bit:

```rust
{{#include ../../../apps/grant/src/perms.rs:presets}}
```

## Revoking

**Revoke** on a key's row builds `RemoveKeys` for that one index, simulates it, and shows it for review in the same way. `RemoveKeys` names only an index, so the page also keeps the pubkey of the row you clicked: it refuses to review or sign if the profile, re-read at each step, now holds another key at that index, and after the transaction confirms it says whether that key is gone. Auth keys cannot be revoked here. Removing a key may move every key after it down one index: the table is re-read after it confirms, so re-check the indexes the agent passes to forge-mcp. Then `sa-forge-signer key destroy` the key file ([Key lifecycle](../key-lifecycle.md)).

## What it checks

- Grants are SAGE-scoped only, from the player-action bits; admin bits are not offered. `DRAIN_SOL_VAULT` (bit 63) is a separate checkbox, off by default: account-creating actions such as creating fleets, crafting and claim stakes need it, and it also lets the key spend the profile's SOL vault.
- The connected wallet must be an unexpired Player Profile key on the profile with `ADD_KEYS` (to grant) or `REMOVE_KEYS` (to revoke), or `AUTH`. A profile whose key threshold is above 1 is refused, since the page signs with one wallet.
- A key already on the profile and unexpired is refused; revoke it first to change its permissions or expiry. The session key may not be the wallet itself.
- Every transaction is built from the form and a fresh read of the profile, never from pasted input. It is simulated before review and rebuilt with a fresh blockhash at signing.
- The signed transaction must carry exactly the message the page built. If the wallet changed it (some wallets add their own instructions), nothing is sent.

## Limits

- **Wallets on Z.ink:** Phantom has no custom networks, so its preview and warnings for a Z.ink transaction may be wrong or missing. The page sends through its own RPC, not the wallet's. Backpack and Solflare accept a custom RPC. Not yet tested with each wallet on Z.ink.
- **Ledger:** a Ledger signs through the wallet it is connected to. Its Solana app does not decode Player Profile instructions, so it needs blind signing, and the device shows only a hash: check the review on the page before you approve.
- **Trust the page you run:** whoever can change the files the page is built from can change what you sign, and with blind signing you would not see it. Build and serve it from a checkout the agent's OS user cannot write, as with the signer's own [isolation](../setup.md#isolation).
- The Player Profile layouts are written by hand from the program's IDL; tests check them against output from its SDK ([`apps/grant/tests/fixtures/`](../../../apps/grant/tests/fixtures/)). Profile discovery searches only the first four key positions.
