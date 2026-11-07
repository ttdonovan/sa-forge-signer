---
description: "What this book is, how to build it, and how to read it: the index first, then only the chapters a task needs"
---

# About this book

The signer's docs in one place: what it checks and how to run it, its configuration, its threat model, the source layout, and the plan. Each chapter includes a doc from where it lives in the repository, so the book and the docs are the same text; change the source, not the chapter.

Install the tools once with `just docs-setup`, then build from the repository root with `just docs`, or `just docs-serve` to read it in a browser. The book is written to `target/docs/html/`, which is not committed.

Agents: read `target/docs/html/llms.txt` first. It lists every chapter with a line on what it holds; then read only the chapters a task needs (each `.md` beside it), or `llms-full.txt` for the whole book at once.

A new doc gets a chapter here and a line in `src/SUMMARY.md`: a chapter is frontmatter with a `description` (one line for the index) and an `{{#include}}` of its source. `just docs-check` fails on a build error, an empty chapter, or a link that leads nowhere.
