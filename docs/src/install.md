---
description: "The install script, installing a verified prebuilt binary or building from a release tag, and checking for and applying updates"
---

# Install and update

Not published on crates.io: install from GitHub, pinned to a release tag (`vX.Y.Z`). Unix only (macOS, Linux).

## Install script

[`scripts/installer/install.sh`](../../scripts/installer/install.sh) installs or updates the signer and the [console](apps/console.md) on the host it runs on, into `~/.local/bin` by default. The signer is two binaries from one release: `sa-forge-signer`, the command line, and `sa-forge-signer-serve`, the [service](serve.md). It downloads the prebuilt signer for the host and verifies it as below (the attestation needs `gh`), or builds it from the tag with cargo when there is no prebuilt binary for the host. It builds the console with cargo from the same tag, and skips it for a tag that does not include it. It also links `sfs` to `sa-forge-signer`, `sfs-serve` to `sa-forge-signer-serve` and `sfs-console` to `sa-forge-signer-console` in the same directory (`--no-alias` skips them), and never replaces an existing file or link of those names. Run it again to update: it skips a signer that is already at that version.

```sh
# From a checkout, or download the script from the tag you want and read it first
scripts/installer/install.sh                    # the latest tag
scripts/installer/install.sh --tag vX.Y.Z --no-console
scripts/installer/install.sh --from-checkout    # build both from this checkout's working tree, not a release
scripts/installer/install.sh --help             # --prefix, --from-source, --console-ref, --skip-attestation, --force
```

It installs binaries only. The config, keys and audit log stay in `~/.config/sa-forge-signer/` (see [Configuration](configuration.md)). Nothing about logging is set at install: tracing is switched on per run with `SA_FORGE_SIGNER_LOG` in the environment of whatever starts the signer, such as your shell, the `serve` service, a container or an MCP client's config ([Tracing](configuration.md#tracing)).

`--uninstall` removes the binaries and their links, and never touches the config, keys or audit log. While key files remain it keeps the signer, which `key destroy` needs, and lists the steps to retire them (see [Key lifecycle](key-lifecycle.md)): revoke each grant, `sa-forge-signer key destroy <name>`, then remove the config directory. Run it again afterwards, or add `--force` to remove the signer anyway.

## Prebuilt binary

Each [release](https://github.com/ttdonovan/sa-forge-signer/releases) carries `sa-forge-signer-vX.Y.Z-<target>.tar.gz` for `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` (static, any distribution), `aarch64-apple-darwin` and `x86_64-apple-darwin`, a `SHA256SUMS` file, and a build provenance attestation. The archive holds both binaries, `sa-forge-signer` and `sa-forge-signer-serve`. Verify before you run it:

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

cargo install --locked --git https://github.com/ttdonovan/sa-forge-signer --tag vX.Y.Z sa-forge-signer
sa-forge-signer --version
```

That installs `sa-forge-signer`, `sa-forge-signer-serve` and `sa-forge-signer-mcp` ([MCP over stdio](serve.md#mcp-over-stdio)), which the prebuilt archive does not carry yet. Install a tag, not a branch: `main` can hold unreleased changes, and a tag is what the [changelog](https://github.com/ttdonovan/sa-forge-signer/blob/main/CHANGELOG.md) describes. `--locked` builds with the dependency versions in `Cargo.lock`, the ones CI tested and audited.

## Updates

Compare `sa-forge-signer --version` (or `cargo install --list`, which shows the installed tag) with the latest tag, read the [changelog](https://github.com/ttdonovan/sa-forge-signer/blob/main/CHANGELOG.md) for anything breaking, then download and verify the new release's binary, or run the same `cargo install` with the new tag (it replaces the installed binary). To be notified of new versions, watch the repository on GitHub for releases only (Watch, Custom, Releases), or subscribe to `https://github.com/ttdonovan/sa-forge-signer/releases.atom`. A running `serve` keeps the old binary until it is restarted.

To build from a clone instead, `cargo build --release --locked` uses the toolchain pinned in `rust-toolchain.toml`.
