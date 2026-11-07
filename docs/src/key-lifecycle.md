---
description: "Creating, granting, funding, using, revoking and destroying a signer key"
---

# Key lifecycle

1. **Create:** `sa-forge-signer key new alice-session --class session` prints only the pubkey; add the printed `[keys.alice-session]` block to the config.
2. **Grant:** build forge-mcp `build_add_profile_key` with that pubkey as `newKey`, scope `sage`, the narrowest permission mask and a short `expireTime`. The profile authority signs it: the human's own wallet ([Path B](setup.md#path-b-the-human-holds-the-wallet)), or the agent's `wallet` key through `sign` (Path A; it needs `confirm` or a testnet burner on `auto`).
3. **Fund:** `sa-forge-signer key fund alice-session --zink 0.05 --from <payer> --out fund.json` builds an unsigned transfer. Path A: `sign --key alice-wallet fund.json`, with the session key listed in the wallet's `transfer_to`. Path B: pay it from the human's wallet with any Solana-compatible tool. A small balance doubles as a spending cap.
4. **Use:** `sign` with the key. A missing or too narrow grant fails the simulation with the program's error.
5. **Revoke:** build `build_remove_profile_key`, signed by the profile authority, then `sa-forge-signer key destroy alice-session`. `destroy` zeroes and deletes the key file, and refuses while the key still holds ZINK unless you pass `--abandon-balance`. The on-chain expiry is the backstop if revocation is forgotten.
