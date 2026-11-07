---
description: "config init and config show, and the annotated example config: cluster, RPC, key and state directories, per-key class, approval, limits and transfer destinations, and serve tokens"
---

# Configuration

The config lives at `~/.config/sa-forge-signer/config.toml` unless `--config` or `SA_FORGE_SIGNER_CONFIG` says otherwise. `sa-forge-signer config init` writes the example below there (mode 0600, in a 0700 directory it creates if needed) and refuses to overwrite an existing file. It has no keys yet: add the block `key new` prints for each key, and `[serve]` if you run `serve`.

`sa-forge-signer config show` prints the config as the signer resolves it: the paths, the RPC, and each key with its effective class, approval, limits and whether its key file exists, defaults filled in, then `[serve]` with each token's name and keys. It reads no key material, prints no token hashes, and calls no RPC.

The example, as `config init` writes it:

```toml
{{#include ../../config.example.toml}}
```
