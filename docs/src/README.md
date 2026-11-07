---
description: "What this book is, how to build it, and how to read it: the index first, then only the chapters a task needs"
---

# About this book

The signer's docs in one place: what it checks, how to install and run it, its configuration, its threat model, the source layout, and the plan. It is published at <https://ttdonovan.github.io/sa-forge-signer/>, built from `main`, so it can describe changes not yet released; the docs for a release are in that tag's `docs/src/`.

Most chapters are written here, in `docs/src/`. A few include a file from where it lives in the repository, so the book and that file are the same text; change the file, not the chapter: Security (`SECURITY.md`), Contributing (`CONTRIBUTING.md`), Plan (`PLAN.md`), Changelog (`CHANGELOG.md`), Local review (`scripts/git-review/README.md`), and the example config in Configuration (`config.example.toml`). The repository's `README.md` is a short front page that links here; it is not a chapter.

## Building it

The book is built with [mdBook](https://rust-lang.github.io/mdBook/), [mdbook-mermaid](https://github.com/badboy/mdbook-mermaid) for the diagrams, and [mdbook-llms](https://github.com/ttdonovan/mdbook-llms). Install the tools once, from GitHub, with `just docs setup`:

```sh
cargo install --locked mdbook --version 0.5.2
cargo install --locked mdbook-mermaid --version 0.17.1
cargo install --locked --git https://github.com/ttdonovan/mdbook-llms --tag v0.4.0
```

Then build from the repository root with `just docs`, or `just docs serve` to read it in a browser. The book is written to `target/docs/html/`, which is not committed.

Agents: read `target/docs/html/llms.txt` first (or the [published one](https://ttdonovan.github.io/sa-forge-signer/llms.txt)). It lists every chapter with a line on what it holds and its size; then read only the chapters a task needs (each `.md` beside it), or `llms-full.txt` for the whole book at once. Its preamble is `src/llms-preamble.md`.

A new doc gets a chapter here and a line in `src/SUMMARY.md`: frontmatter with a `description` (one line for the index), then the text, or an `{{#include}}` of its source. `just docs check` fails on a build error, an empty chapter, or a link that leads nowhere.
