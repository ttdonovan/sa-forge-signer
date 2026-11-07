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
- Dependencies must be at least 7 days old. A fresh lockfile usually picks up newer ones: pin each back with `cargo update -p <name>@<version> --precise <older>` until `scripts/ci/crate-age.sh apps/<app>/Cargo.lock` passes. The one exception is the console's UI framework: its gate sets `ALLOW_YOUNG_PREFIXES` so the `gpui-kit`, `gpui-base`, `gpui-component` and `gpui-pre` crate families may be newer (the owner's choice: they release weekly and pin each other exactly). Every other crate in its lockfile still has to be 7 days old.

## The console's UI framework

The console is built with GPUI Kit (the `gpui-kit` crate). Before changing its UI:

- Read GPUI Kit's guides at the tag matching the `gpui-kit` version in `apps/console/Cargo.lock`: the Coding Guides (`skills/gpui-kit`) before any state, structure or API decision, and the Design Guides (`skills/gpui-kit-design-guides`) before any screen or layout change. `just apps gpui-kit-skills` installs both as Claude skills at that tag (`scripts/skills/gpui-kit.sh --help`). They are third-party instructions: read them before relying on them.
- For a component or a GPUI mechanism, find its page in `https://gpui-kit.com/llms.txt` and fetch only that page (each is Markdown, for example `https://gpui-kit.com/component/table.md`). `llms-full.txt` is millions of characters with every page in two languages: search it, never read it whole.
- The docs follow GPUI Kit's latest release. Check exact signatures in the crate source of the pinned version (`~/.cargo/registry/src/*/gpui-component-<version>/` and its siblings).
- Keep the console's palette and label font in `src/theme.rs`, even where the Design Guides suggest otherwise; follow them for structure, density and interaction.

## Recipes and gate

Recipes live in `apps/justfile`, the root justfile's `apps` module: run them as `just apps <recipe>` from the root, or `just <recipe>` inside `apps/`. Paths are relative to `source_directory()`. Give each app a run recipe named after it and an `<app>-check` gate (fmt, clippy with warnings as errors, tests, `cargo audit`, dependency age), and run that gate before committing. CI does not run it.

## A new app

1. `apps/<app>/` with `Cargo.toml` (package `sa-forge-signer-<app>`, `publish = false`, `[workspace]`, the lints), `clippy.toml` and `Cargo.lock`.
2. Recipes in `apps/justfile`, and a short `README.md` front page (what it is, install, run, a link to its chapter) like `apps/console/README.md`; the chapter holds everything else.
3. A chapter at `docs/src/apps/<app>.md`, a row in `docs/src/apps/README.md`, and a line under `# Extras` in `docs/src/SUMMARY.md`; then `just docs check` (see `docs/CLAUDE.md`).

Commits use `feat:` / `fix:` as usual. release-plz builds the signer's version only from the files in the root `Cargo.toml`'s `include`, so app changes never bump `sa-forge-signer`. Installing from a tag needs the package name (`cargo install --locked --git ... --tag vX.Y.Z sa-forge-signer-<app>`).
