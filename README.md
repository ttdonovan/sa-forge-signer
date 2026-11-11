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
cargo install --locked --git https://github.com/ttdonovan/sa-forge-signer --tag vX.Y.Z sa-forge-signer
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
```

## Using it from an agent

An agent does not run the command line above: it talks to the long-running service, `sa-forge-signer-serve`, with a bearer token that lists the keys it may use. [Getting started](https://ttdonovan.github.io/sa-forge-signer/getting-started.html) walks through it end to end, and [Serving over HTTP and MCP](https://ttdonovan.github.io/sa-forge-signer/serve.html) is the reference for the routes, the tools and the token.

```sh
# Create a token; prints its sha256 and a [[serve.tokens]] snippet for the config
sa-forge-signer token new agent --out ~/.config/sa-forge-signer/agent.token

# Run the service as the signer's user, not the agent's: HTTP and MCP on 127.0.0.1:8790
sa-forge-signer-serve

# Then add it to the agent's MCP client, over HTTP...
claude mcp add --transport http sa-forge-signer http://127.0.0.1:8790/mcp \
  --header "Authorization: Bearer $(cat ~/.config/sa-forge-signer/agent.token)"

# ...or over stdio, for a client that only starts its servers as child processes
claude mcp add sa-forge-signer -- sa-forge-signer-mcp --token-file ~/.config/sa-forge-signer/agent.token

# Optional: install the skill that teaches the agent the tools and what to do with each outcome
sa-forge-signer-mcp skill install
```

The agent then passes each forge-mcp `build_*` payload to the `check` tool, then `sign`. The service also serves that guidance as MCP resources, starting at `sa-forge-signer://llms.txt`. `sa-forge-signer-mcp` is a client of the running service: it holds the token, never a key. The prebuilt archive does not carry it yet; `cargo install` builds it with the other two.

## Docs

| Chapter | For |
|---|---|
| [How it works](https://ttdonovan.github.io/sa-forge-signer/how-it-works.html) | The checks it runs before signing, and what it leaves to the chain |
| [Install and update](https://ttdonovan.github.io/sa-forge-signer/install.html) | Verified binaries, building from a tag, checking for updates |
| [Getting started](https://ttdonovan.github.io/sa-forge-signer/getting-started.html) | From install to an agent signing over MCP, with the full config for each setup |
| [Choosing a setup](https://ttdonovan.github.io/sa-forge-signer/setup.html) | Who holds the wallet, what each setup can lose, isolation |
| [Checking and signing](https://ttdonovan.github.io/sa-forge-signer/check-and-sign.html) | The CLI, the JSON report, outcomes and exit codes, retrying safely |
| [Key lifecycle](https://ttdonovan.github.io/sa-forge-signer/key-lifecycle.html) | Create, grant, fund, use, revoke |
| [Configuration](https://ttdonovan.github.io/sa-forge-signer/configuration.html) | The annotated example config |
| [Serving over HTTP and MCP](https://ttdonovan.github.io/sa-forge-signer/serve.html) | The service, its routes and MCP tools, tokens, shutdown, adding it to Claude Code over HTTP or stdio, the agent skill |
| [Design decisions](https://ttdonovan.github.io/sa-forge-signer/dev/design.html) | Why three binaries, why the stdio MCP is a client of the service, why not an MCP SDK |
| [Running in Docker](https://ttdonovan.github.io/sa-forge-signer/docker.html) | The image and its mounts |
| [Security](https://ttdonovan.github.io/sa-forge-signer/security.html) | Threat model, limits, reporting a vulnerability |
| [Contributing](https://ttdonovan.github.io/sa-forge-signer/dev/contributing.html) | The check gate, commits, releases |

## License

Dual-licensed under the [Unlicense](https://github.com/ttdonovan/sa-forge-signer/blob/main/UNLICENSE) or [MIT](https://github.com/ttdonovan/sa-forge-signer/blob/main/LICENSE-MIT), at your option. Contributions are accepted under the same terms (see [COPYING](https://github.com/ttdonovan/sa-forge-signer/blob/main/COPYING)).
