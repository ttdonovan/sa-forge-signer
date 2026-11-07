# Contributing

## Prerequisites

- Rust, at the version pinned in `rust-toolchain.toml`
- [just](https://github.com/casey/just)
- [shellcheck](https://www.shellcheck.net/) and [cargo-audit](https://github.com/rustsec/rustsec/tree/main/cargo-audit), for `just check`
- [mdBook](https://rust-lang.github.io/mdBook/) 0.5, [mdbook-mermaid](https://github.com/badboy/mdbook-mermaid) and [mdbook-llms](https://github.com/ttdonovan/mdbook-llms), for the docs: `just docs setup` installs all three

## Workflow

Run `just` to list recipes. Before opening a pull request, run `just check` (the same gate as CI: fmt, clippy with warnings as errors, tests, shellcheck, advisories, dependency age), and `just docs check` if a doc changed. Keep `PLAN.md` in step with the work it tracks.

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
