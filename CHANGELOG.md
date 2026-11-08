# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.1](https://github.com/ttdonovan/sa-forge-signer/compare/v0.4.0...v0.4.1) - 2026-11-07

### Added

- add an optional explorer_url to the config, defaulting to the cluster's explorer, used for the explorer link in sign reports and shown by config show.
- make key import of the same keypair a no-op that prints its config block again, still refusing a different key.
- point config init and config show at key import as well as key new.
- add key import, which copies an existing Solana CLI keypair file into the key store and prints its config block.
- add config init, which writes the annotated example config, and config show, which prints the config as the signer resolves it.

### Other

- replace the bare allow on unless_stale with an expect that gives its reason.
- move the Working on it chapters into docs/src/dev.
- add a Getting started chapter with the full config for each setup, the minimal serve table, and a table of every setting and its default.
- name the package in the cargo install command, which cargo requires once the repository holds a second binary package.
- split the README into book chapters, and slim the README to a front page.
- build, attest and attach prebuilt binaries to each release.
- list the crate's files in Cargo.toml, so CI and docs-only changes do not trigger a release.
- group only minor and patch Dependabot updates, so each major bump gets its own PR.
- *(deps)* bump Swatinem/rust-cache

## [0.4.0](https://github.com/ttdonovan/sa-forge-signer/releases/tag/v0.4.0) - 2026-11-07

### Fixed

- *(deps)* update rustls-webpki 0.103.15 -> 0.103.16 and webpki-roots 1.0.9 -> 1.0.11 for RUSTSEC-2026-0382 and RUSTSEC-2026-0381.

### Other

- add weekly Dependabot updates for actions and cargo with a 7-day cooldown.
- release with release-plz in git-only mode, and document Conventional Commits and what counts as breaking.
- check the book on pull requests and deploy it to GitHub Pages from main.
- build the docs as an mdBook with llms.txt for agents, and point agents at it from CLAUDE.md.
- rustls 0.23.45 -> 0.23.46 for RUSTSEC-2026-0346..0349, allowed by a dated exception in the crate-age check (owner's exception to the 7-day minimum age).
- Refuse a sign whose blockhash is already past the finalized height, after one fresh re-sign: it can never land, so nothing is sent and a retry is safe.
- Bump the version to 0.4.0 for the security review's breaking changes.
- Require the real System and token programs in ATA creates; check token authorization before the key lock.
- Pinchy's re-review of 2f942bf: the two P2 follow-ups
- Close Pinchy's re-review of 6d58901 (P1-1..3, P2)
- Hardening and documentation from the review (P2)
- Keep the profile authority behind a person at the terminal (review P1)
- Serialize signs per key; reserve spend before sending (review P1)
- Simulate the post-approval transaction before sending it (review P1)
- Prove expiry against the finalized tip; bound every sign (review P1)
- Refuse top-level account creation and token-program calls (review P0)
- Add sa-forge-signer 0.3.3: a thin signer for SAGE on Z.ink testnet with structural checks, simulation, confirmation, a file key store, a hash-chained audit log, and serve with HTTP and MCP endpoints.
