---
description: "Extras built beside the signer under apps/: what they are, and how they relate to the signer's releases and gate"
---

# Apps

Extras that sit beside the signer under `apps/`, each its own crate with its own workspace and `Cargo.lock`. They are not part of the signer: `just check`, CI, the release binaries, the Docker image and the signer's version leave them out, and a change to one does not bump `sa-forge-signer`.

None of them holds keys or signs with the signer's keys. The grant page has the human's own browser wallet sign; it holds no keys itself. Each runs as the human, not the agent, and the [isolation rule](../setup.md#isolation) still applies: nothing here should give the agent's OS user a way into the signer's keys, config or state.

| App | For |
|---|---|
| [Console](console.md) | A read-only desktop view of a running signer: `serve` health, the audit log and per-key limits |
| [Grant page](grant.md) | A local web page where the profile wallet grants and revokes a session key (Path B) |

Install an app from a release tag by naming its package (`cargo install --locked --git https://github.com/ttdonovan/sa-forge-signer --tag vX.Y.Z <package>`), or from a checkout with `cargo install --locked --path apps/<app>`.
