---
description: "From install to an agent signing over MCP: the short path with the install script's --setup, then by hand config init, a key, a serve token and claude mcp add, with the full config.toml for a testnet wallet setup and for the recommended session-key setup"
---

# Getting started

One pass from nothing to an agent signing through `serve`, first the short path with the install script's `--setup`, then each step by hand. It runs everything as your own OS user, which suits testnet: an agent running as you can read the keys (see [Isolation](setup.md#isolation) before using anything of value). [Choosing a setup](setup.md) explains what each setup below can lose.

## The short path

```sh
scripts/installer/install.sh --setup            # install, create the agent token, add the MCP server to Claude Code, install the skill
sfs config init
sfs key new alice-session --class session       # or a testnet wallet, as in step 3
```

Then, as in step 3, add the key's block and the `[[serve.tokens]]` block that `--setup` showed to `config.toml`, with `keys = ["alice-session"]`, and grant and fund the key. Check it with `sfs config show`, leave `sfs-serve` running, and restart Claude Code. [Connecting Claude Code](install.md#connecting-claude-code) says what `--setup` does and how `--uninstall` undoes it. The steps below are the same by hand, and the reference for each part: after `--setup`, skip `token new` in step 3 and `claude mcp add` in step 4.

## 1. Install

```sh
scripts/installer/install.sh        # from a checkout; see Install and update for other ways
```

It installs `sa-forge-signer`, `sa-forge-signer-serve`, `sa-forge-signer-mcp` and `sa-forge-signer-console` into `~/.local/bin` and links `sfs`, `sfs-serve`, `sfs-mcp` and `sfs-console` to them ([Install and update](install.md)). The commands below use `sfs`.

## 2. Create the config

```sh
sfs config init     # writes ~/.config/sa-forge-signer/config.toml, with no keys yet
sfs config show
```

Keys go in `~/.config/sa-forge-signer/keys/` and the audit log in `~/.config/sa-forge-signer/state/` (see [Configuration](configuration.md)).

## 3. Add a key and a token

Pick one setup. Each ends with the whole block to add to `config.toml`; everything else in the file can stay as `config init` wrote it.

### Testnet: the agent holds a throwaway wallet

The agent's wallet is the profile authority and signs everything through `serve`, unattended. Use this only for a testnet burner: anything that controls the agent can sign as the profile authority.

```sh
sfs key new alice-wallet --class wallet          # or, for a keypair file you have:
sfs key import alice-wallet --class wallet --from ~/path/to/keypair.json
sfs token new agent --out ~/.config/sa-forge-signer/agent.token
```

Fund the wallet with testnet ZINK: it pays the fees. Then add:

```toml
[keys.alice-wallet]
class = "wallet"
pubkey = "<pubkey printed by key new or key import>"
approval = "auto"          # sign without a person at a terminal
allow_unattended = true    # lets a wallet key be "auto" and sit on a serve token; throwaway testnet keys only

[serve]
[[serve.tokens]]
name = "agent"
sha256 = "<hash printed by token new>"
keys = ["alice-wallet"]
```

Both extra lines are needed. A `wallet` key defaults to `approval = "confirm"`, which asks on the terminal running `serve` for every signature and refuses when there is none. And the config refuses to load a wallet key that is `auto` or on a token without `allow_unattended = true`: the signer cannot see what a profile grant gives away.

### Recommended: you hold the wallet (Path B)

Your own wallet stays outside the signer and grants the agent a narrow, expiring session key.

```sh
sfs key new alice-session --class session
sfs token new agent --out ~/.config/sa-forge-signer/agent.token
```

Grant the key on your profile with forge-mcp `build_add_profile_key` (the narrowest permission mask and a short expiry), signed by your wallet, and send it a small amount of ZINK for fees ([Key lifecycle](key-lifecycle.md)). Then add:

```toml
[keys.alice-session]
class = "session"
pubkey = "<pubkey printed by key new>"

[serve]
[[serve.tokens]]
name = "agent"
sha256 = "<hash printed by token new>"
keys = ["alice-session"]
```

A session key defaults to `approval = "auto"`, so nothing else is needed.

Path A as [Choosing a setup](setup.md#path-a-the-agent-holds-the-wallet) describes it combines the two: the wallet key stays on `confirm` and off the token, and you add a session key as above, granted and funded by the wallet through `sfs sign --key alice-wallet` at a terminal.

## 4. Check, run and connect

```sh
sfs config show
```

For the testnet setup it ends like this (the session setup shows `class: session`, `approval: auto` and no `allow_unattended` line):

```text
[keys.alice-wallet]
  pubkey:         6YAf723681zcJjFrd9UDJCtymiTGov8FL9Ed6XvA2evw
  class:          wallet
  approval:       auto
  key file:       present
  profile:        -
  rate limit:     30 per minute
  daily cap:      50000000 lamports (0.05 ZINK)
  transfer_to:    -
  extra_programs: -
  allow_unattended: true (throwaway testnet keys only)

[serve]
  listen:         127.0.0.1:8790
  allowed_hosts:  127.0.0.1, localhost
  token:          agent (keys: alice-wallet)
```

`key file: missing` means the key is in the config but not in the key directory. The daily cap and rate limit are the defaults; raise them per key if the agent hits them ([Configuration](configuration.md#settings)).

```sh
sfs-serve                                   # leave it running; 127.0.0.1:8790
claude mcp add --transport http sa-forge-signer http://127.0.0.1:8790/mcp \
  --header "Authorization: Bearer $(cat ~/.config/sa-forge-signer/agent.token)"
```

The agent passes each forge-mcp `build_*` payload to the `check` tool, then `sign` ([Serving over HTTP and MCP](serve.md)). To watch it, run `sfs-console` in another window ([Console](apps/console.md)).
