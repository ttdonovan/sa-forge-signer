# sa-forge-signer-grant

A local web page where the profile authority's own wallet (Phantom, Backpack, Solflare, or a Ledger through one of them) grants and revokes an [sa-forge-signer](../../README.md) session key on a SAGE profile. It holds no keys: the wallet signs. Z.ink testnet only. Early spike.

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk

# From the repository root; then open http://127.0.0.1:8791 in the browser that has the wallet
just apps grant
```

**Docs:** [Grant page](https://ttdonovan.github.io/sa-forge-signer/apps/grant.html) in the book (source: [`docs/src/apps/grant.md`](../../docs/src/apps/grant.md)) covers what it checks, what it refuses, and its limits.
