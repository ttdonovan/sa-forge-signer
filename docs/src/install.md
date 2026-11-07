---
description: "Installing a verified prebuilt binary or building from a release tag, and checking for and applying updates"
---

# Install and update

Not published on crates.io: install from GitHub, pinned to a release tag (`vX.Y.Z`). Unix only (macOS, Linux).

## Prebuilt binary

Each [release](https://github.com/ttdonovan/sa-forge-signer/releases) carries `sa-forge-signer-vX.Y.Z-<target>.tar.gz` for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` (static, any distribution), `aarch64-apple-darwin` and `x86_64-apple-darwin`, a `SHA256SUMS` file, and a build provenance attestation. Verify before you run it:

```sh
gh release download vX.Y.Z --repo ttdonovan/sa-forge-signer --pattern '*-aarch64-apple-darwin.tar.gz' --pattern SHA256SUMS
shasum -a 256 -c --ignore-missing SHA256SUMS
gh attestation verify sa-forge-signer-vX.Y.Z-aarch64-apple-darwin.tar.gz --repo ttdonovan/sa-forge-signer \
  --signer-workflow ttdonovan/sa-forge-signer/.github/workflows/release-binaries.yml
tar -xzf sa-forge-signer-vX.Y.Z-aarch64-apple-darwin.tar.gz
```

The attestation proves the archive was built by this repository's `release-binaries.yml` workflow, as run from `main`; that workflow checks out the tag and refuses one that does not match the version in `Cargo.toml`. The checksum alone only proves the archive matches the file next to it. The macOS binaries are not signed or notarized: a copy downloaded with a browser is quarantined by Gatekeeper (`gh` and `curl` do not set the quarantine flag).

## From source

Needs Rust 1.89 or later.

```sh
# Find the latest release tag
git ls-remote --tags --refs --sort=-version:refname https://github.com/ttdonovan/sa-forge-signer 'v*' | head -1

cargo install --locked --git https://github.com/ttdonovan/sa-forge-signer --tag vX.Y.Z
sa-forge-signer --version
```

Install a tag, not a branch: `main` can hold unreleased changes, and a tag is what the [changelog](https://github.com/ttdonovan/sa-forge-signer/blob/main/CHANGELOG.md) describes. `--locked` builds with the dependency versions in `Cargo.lock`, the ones CI tested and audited.

## Updates

Compare `sa-forge-signer --version` (or `cargo install --list`, which shows the installed tag) with the latest tag, read the [changelog](https://github.com/ttdonovan/sa-forge-signer/blob/main/CHANGELOG.md) for anything breaking, then download and verify the new release's binary, or run the same `cargo install` with the new tag (it replaces the installed binary). To be notified of new versions, watch the repository on GitHub for releases only (Watch, Custom, Releases), or subscribe to `https://github.com/ttdonovan/sa-forge-signer/releases.atom`. A running `serve` keeps the old binary until it is restarted.

To build from a clone instead, `cargo build --release --locked` uses the toolchain pinned in `rust-toolchain.toml`.
