# Contributing

## Prerequisites

- Rust, at the version pinned in `rust-toolchain.toml`
- [just](https://github.com/casey/just)
- [shellcheck](https://www.shellcheck.net/) and [cargo-audit](https://github.com/rustsec/rustsec/tree/main/cargo-audit), for `just check`
- [mdBook](https://rust-lang.github.io/mdBook/) 0.5, [mdbook-mermaid](https://github.com/badboy/mdbook-mermaid) and [mdbook-llms](https://github.com/ttdonovan/mdbook-llms), for the docs: `just docs setup` installs all three

## Workflow

Run `just` to list recipes. Before opening a pull request, run `just check` (the same gate as CI: fmt, clippy with warnings as errors, tests, shellcheck, advisories, dependency age), and `just docs check` if a doc changed. Keep `PLAN.md` in step with the work it tracks.

## Code style

The tools enforce most of it: `cargo fmt`, and clippy with `pedantic` and `nursery` denied and the panic rules in `Cargo.toml`'s `[lints]` (no `unwrap`, `expect`, `panic!`, indexing or slicing outside tests, no `as` casts, checked arithmetic, `unsafe_code` forbidden). The rest is by convention:

- **Errors:** return `anyhow::Result`; add context to every file, network and parse error (`.with_context(|| format!("reading {}", path.display()))`) so a message names what failed.
- **Lints:** fix the code, for example split a function that is too long, rather than silencing the lint. Where that would make the code worse, or the lint is a false positive or fires on generated code, use `#[expect(lint, reason = "...")]` on the one item, never a bare `#[allow]`.
- **Control flow:** `match` or `let ... else` instead of checking and then unwrapping; a plain `for` loop when an iterator chain would be harder to read; an enum instead of a `bool` parameter whose meaning is not obvious at the call site.
- **Names:** whole words, not abbreviations, except the usual short ones (`cfg`, `tx`, `ix`).
- **Modules:** each module's `//!` comment says what it owns and is the authority if the [code map](https://ttdonovan.github.io/sa-forge-signer/dev/code-map.html) drifts. Code that mirrors other code (as the console mirrors the audit log) says so and has a test against real output.
- **Comments:** short, for what the code cannot say: why, an invariant, or a dated observation.
- **Tests:** for every check and every piece of logic; a refusal is tested as well as the pass.

For further reading, Google's [Rust style guide](https://google.github.io/styleguide/rustguide.html) and the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/) are close to this, with two differences: we deny `expect` as well as `unwrap` outside tests, and the public interface is the one under Versioning, not a library API.

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org/): prefix each message with its type, write the rest as a sentence, and end it with a full stop.

- `feat:` a new capability (a check, a command, a config key, a route)
- `fix:` a bug fix
- `docs:`, `refactor:`, `test:`, `build:`, `ci:`, `chore:` for the rest

Mark a breaking change with `!` (`feat!:`, `fix!:`) and say what breaks in the body. For example: `fix!: refuse a sign whose blockhash is already past the finalized height.`

## Versioning

The crate follows [Semantic Versioning](https://semver.org/). Its public interface is what a caller or operator depends on, so a change to any of these is breaking:

- the CLI: commands, flags, and their defaults
- the JSON report: field names, types, and `outcome` values
- exit codes
- `config.toml`: keys, their meaning, and their defaults
- the payload format the signer accepts
- `serve`: HTTP routes, MCP tools, and auth
- the audit log format

So is a check that refuses what was signed before. Adding an optional field, key, or command is not breaking.

While the version is `0.x`, a breaking change bumps the minor version (`0.4.0` to `0.5.0`) and anything else bumps the patch version.

## Releases

Releases are made by [release-plz](https://release-plz.dev/) from the commit messages. The crate is not published to crates.io (`publish = false`); release-plz runs in git-only mode, reading the current version from the `vX.Y.Z` tags. On each push to `main`, it opens or updates a release PR that bumps the version in `Cargo.toml` and updates `CHANGELOG.md`. Review both in that PR; merging it tags `vX.Y.Z` and creates a GitHub Release. The `Release binaries` workflow then builds the binaries for that tag, attests them, and attaches them with a `SHA256SUMS` file to the release. To build them for an existing tag (or rebuild them), run it by hand from the Actions tab with the tag.

Install a release from its tag:

```sh
cargo install --locked --git https://github.com/ttdonovan/sa-forge-signer --tag vX.Y.Z sa-forge-signer
```
