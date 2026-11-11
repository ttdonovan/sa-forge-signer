# sa-forge-signer tools

Every tool acts only on the keys the server's token lists. `check` and `sign` return the JSON report below as text; a tool result is marked as an error when the outcome is anything but `ok` or `confirmed`.

- `check(payload, key?)`: Run every check and a simulation on a payload. Never signs. `payload` is the object a forge-mcp `build_*` call returned, unchanged. `key` is a configured key name; left out, the key matching the payload's fee payer is used.
- `sign(payload, key?, intent?)`: Check, simulate, sign, send and confirm a payload. `intent` is one line on why, recorded in the audit log.
- `key_show(name)`: A key's `pubkey`, `class`, `approval` (`auto`, `confirm` or `deny`), `profile`, `balance_lamports`, `halted` (`null`, or `since`, `by` and `reason`) and `policy` (`digest`, `approval_required`, `approved`).
- `halt(reason, key?)`: Stop all signing with a key, or with every key the token lists, until a person resumes it at the signer's terminal. Returns `{"halted": [<keys>]}`. There is no resume tool.

## The report

| Field | Holds |
|---|---|
| `outcome` | `ok`, `confirmed`, `refused`, `simulation_failed`, `failed` or `unknown` |
| `key` | The name of the key the payload was checked against or signed with |
| `failed_check` | On `refused`: the check that failed |
| `detail` | Why, in words: a refusal's reason, or what to do after `unknown` |
| `checks` | The checks that passed, in order |
| `sign_preview` | From `check` only: `refusals`, the checks `sign` would refuse now (`halt`, `policy`, `approval`, `limits`) as `{"check", "detail"}`, and `needs_approval`, `true` for a key that prompts a person |
| `signature`, `slot`, `explorer` | Once sent: the transaction's signature, the slot it landed in, and a link to it |
| `program_error`, `error`, `logs` | The program's own message, the raw error and the logs, from a failed simulation or a transaction that landed with an error |
| `compute_units`, `fee`, `balance_change` | Simulated for `check`; from the transaction's own metadata once it landed. Lamports; `balance_change` is negative for a spend |
| `findings` | Present only when the signer has an instruction table: `{"check", "detail"}` notes on an instruction it does not list, a wrong account count, or a profile that is not the key's |
| `summary`, `warnings` | The payload's own summary and warnings, written by the builder and not verified |
| `payload_hash` | The sha256 the audit log records for this payload |
