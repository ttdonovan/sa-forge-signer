---
description: "The non-root image, the config, keys and state mounts, one-shot and service runs, and keeping keys from the agent"
---

# Running in Docker

The `Dockerfile` builds a small non-root image (uid 10001) with base images pinned by digest. It holds no keys or config; mount them at run time:

| Mount | Path in the container | Notes |
|---|---|---|
| config | `/etc/sa-forge-signer/config.toml` (read-only) | set `key_dir = "/run/keys"` and `state_dir = "/var/lib/sa-forge-signer"`; for `serve`, `listen = "0.0.0.0:8790"` |
| keys | `/run/keys` (read-only) | a 0700 directory of 0600 key files, readable by uid 10001; nothing else should mount it |
| state | `/var/lib/sa-forge-signer` | the audit log; keep it across restarts |

```sh
docker build -t sa-forge-signer .

# One-shot: check or sign a payload from stdin
docker run --rm -i --read-only --cap-drop ALL \
  -v "$PWD/config.toml:/etc/sa-forge-signer/config.toml:ro" -v "$PWD/keys:/run/keys:ro" \
  -v "$PWD/state:/var/lib/sa-forge-signer" sa-forge-signer check - < payload.json

# Service (the default command): publish on loopback only; allow at least 230 s to stop so signs drain
docker run -d --name sa-forge-signer --read-only --cap-drop ALL --stop-timeout 230 \
  -p 127.0.0.1:8790:8790 \
  -v "$PWD/config.toml:/etc/sa-forge-signer/config.toml:ro" -v "$PWD/keys:/run/keys:ro" \
  -v "$PWD/state:/var/lib/sa-forge-signer" sa-forge-signer
```

The signer talks only to the RPC in `rpc_url` (by default Z.ink testnet). An agent can use a forge-mcp running anywhere, local or hosted: only unsigned payloads travel to the signer, and the keys never leave this container. The [isolation rule](setup.md#isolation) still applies: the agent must not be able to read the keys directory or the config, and must not be able to use Docker, which can mount them. On macOS, bind-mounted files stay owned by your macOS user, so an agent running as you can read the keys (see [Security](security.md)).
