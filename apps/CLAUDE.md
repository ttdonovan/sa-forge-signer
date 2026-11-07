# apps/

Extras that sit beside the signer: tools a human runs to watch or work with it. Each app is its own crate, not part of `sa-forge-signer`.

## Rules every app keeps

- **Never weaken the signer's isolation.** An app runs as the human, never the agent. It does not read keys, sign, or hold anything that can sign. It reads the signer's state and config read-only (or through `serve`'s routes) and never writes, locks or truncates them: write access to `state_dir` can rewrite the audit chain. Nothing an app adds may give the agent's OS user a path to the keys, config or state, or a way to approve a signature.
- **Approval does not go through a terminal pane or anything the agent's user can script.** See "Typing into the approval terminal" in `SECURITY.md`.
- **Stay out of the signer's build.** Each app has its own `[workspace]` table and `Cargo.lock`, so it is not a member of the root package, and its dependencies stay out of `just check`, CI, the release binaries and the Docker image. Do not add an app to any of them.
- **Package names start with `sa-forge-signer-`** (for example `sa-forge-signer-console`), so they are not confused with other SAGE forge projects.

## Code

- Same standards as the signer: Rust edition 2024, the root's `[lints]` block (no panics, no `as` casts, checked arithmetic) copied into the app's `Cargo.toml`, a copy of `clippy.toml`, `cargo fmt`, and tests for logic.
- When an app mirrors signer logic (the console mirrors the audit `Entry` and `limits`), say so in the module doc, keep it in step with `src/`, and keep a test that parses output written by the real signer, as `apps/console/tests/fixtures/` does.
- Dependencies must be at least 7 days old. A fresh lockfile usually picks up newer ones: pin each back with `cargo update -p <name>@<version> --precise <older>` until `scripts/ci/crate-age.sh apps/<app>/Cargo.lock` passes.

## Recipes and gate

Recipes live in `apps/justfile`, the root justfile's `apps` module: run them as `just apps <recipe>` from the root, or `just <recipe>` inside `apps/`. Paths are relative to `source_directory()`. Give each app a run recipe named after it and an `<app>-check` gate (fmt, clippy with warnings as errors, tests, `cargo audit`, dependency age), and run that gate before committing. CI does not run it.

## A new app

1. `apps/<app>/` with `Cargo.toml` (package `sa-forge-signer-<app>`, `publish = false`, `[workspace]`, the lints), `clippy.toml` and `Cargo.lock`.
2. Recipes in `apps/justfile`, and a short `README.md` front page (what it is, install, run, a link to its chapter) like `apps/console/README.md`; the chapter holds everything else.
3. A chapter at `docs/src/apps/<app>.md`, a row in `docs/src/apps/README.md`, and a line under `# Extras` in `docs/src/SUMMARY.md`; then `just docs check` (see `docs/CLAUDE.md`).

Commits use `feat:` / `fix:` as usual. release-plz builds the signer's version only from the files in the root `Cargo.toml`'s `include`, so app changes never bump `sa-forge-signer`. Installing from a tag needs the package name (`cargo install --locked --git ... --tag vX.Y.Z sa-forge-signer-<app>`).
