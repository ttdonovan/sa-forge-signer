# sa-forge-signer-console

A read-only desktop view, built with GPUI Kit, of a running [sa-forge-signer](../../README.md): `serve` health, the audit log and per-key limits. It never signs, approves or writes anything. Early spike.

```sh
# From a release tag, or from a checkout
cargo install --locked --git https://github.com/ttdonovan/sa-forge-signer --tag vX.Y.Z sa-forge-signer-console
cargo install --locked --path apps/console

# Or run it from the repository root
just apps console --config ./config.toml
```

**Docs:** [Console](https://ttdonovan.github.io/sa-forge-signer/apps/console.html) in the book (source: [`docs/src/apps/console.md`](../../docs/src/apps/console.md)) covers flags, what it shows, access, and watching a signer in Docker.
