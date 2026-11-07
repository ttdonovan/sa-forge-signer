# sa-forge-signer

A thin, self-hosted signer for agents that play SAGE C4 on Z.ink. The agent builds unsigned payloads with forge-mcp `build_*` methods; `sa-forge-signer` checks, simulates, signs, sends and confirms them with keys the agent itself cannot read.

**Unofficial.** Not affiliated with or endorsed by Star Atlas or its developers. Z.ink testnet (SAGE C4 PTR) only; there is no mainnet profile.

**Status:** early. Read the code before trusting it with anything.

**Docs:** <https://ttdonovan.github.io/sa-forge-signer/>. Agents: start at [llms.txt](https://ttdonovan.github.io/sa-forge-signer/llms.txt). The site follows `main`; a release's docs are in its tag's `docs/src/`.

## Before you use it

- **Isolate it.** The signer only protects anything if the agent cannot read its keys or config. Run it as a different OS user or in a container, with the key directory and config owned by that user. If the agent can read the key file, the checks are advice, not a control.
- **Hold the wallet yourself for anything of value.** Set up the profile with your own wallet and grant the agent a narrow, expiring session key (Path B). Letting the agent hold the wallet (Path A) suits testnet burners. See [Choosing a setup](https://ttdonovan.github.io/sa-forge-signer/setup.html).

## Install

Not published on crates.io. Each [release](https://github.com/ttdonovan/sa-forge-signer/releases) has prebuilt binaries for Linux (static, x86_64 and aarch64) and macOS, with checksums and a build provenance attestation; or build a release tag from source (Rust 1.89 or later):

```sh
cargo install --locked --git https://github.com/ttdonovan/sa-forge-signer --tag vX.Y.Z
```

[Install and update](https://ttdonovan.github.io/sa-forge-signer/install.html) covers verifying a binary, finding the latest tag, and updating.

## Quick start

```sh
# Write the annotated example config to ~/.config/sa-forge-signer/config.toml
sa-forge-signer config init

# Generate a key; prints only the pubkey and a config snippet
sa-forge-signer key new alice-session --class session

# Paste that snippet into the config (check it with `sa-forge-signer config show`), then grant
# the key on the profile with forge-mcp build_add_profile_key, signed by the profile authority.

sa-forge-signer check payload.json                  # checks + simulation, never signs
sa-forge-signer sign --intent "scan round" payload.json
sa-forge-signer serve                               # HTTP and MCP on 127.0.0.1:8790
```

## Docs

| Chapter | For |
|---|---|
| [How it works](https://ttdonovan.github.io/sa-forge-signer/how-it-works.html) | The checks it runs before signing, and what it leaves to the chain |
| [Install and update](https://ttdonovan.github.io/sa-forge-signer/install.html) | Verified binaries, building from a tag, checking for updates |
| [Choosing a setup](https://ttdonovan.github.io/sa-forge-signer/setup.html) | Who holds the wallet, what each setup can lose, isolation |
| [Checking and signing](https://ttdonovan.github.io/sa-forge-signer/check-and-sign.html) | The CLI, the JSON report, outcomes and exit codes, retrying safely |
| [Key lifecycle](https://ttdonovan.github.io/sa-forge-signer/key-lifecycle.html) | Create, grant, fund, use, revoke |
| [Configuration](https://ttdonovan.github.io/sa-forge-signer/configuration.html) | The annotated example config |
| [Serving over HTTP and MCP](https://ttdonovan.github.io/sa-forge-signer/serve.html) | `serve`, tokens, shutdown, adding it to Claude Code |
| [Running in Docker](https://ttdonovan.github.io/sa-forge-signer/docker.html) | The image and its mounts |
| [Security](https://ttdonovan.github.io/sa-forge-signer/security.html) | Threat model, limits, reporting a vulnerability |
| [Contributing](https://ttdonovan.github.io/sa-forge-signer/contributing.html) | The check gate, commits, releases |

## License

Dual-licensed under the [Unlicense](https://github.com/ttdonovan/sa-forge-signer/blob/main/UNLICENSE) or [MIT](https://github.com/ttdonovan/sa-forge-signer/blob/main/LICENSE-MIT), at your option. Contributions are accepted under the same terms (see [COPYING](https://github.com/ttdonovan/sa-forge-signer/blob/main/COPYING)).
