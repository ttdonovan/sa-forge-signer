# docs/

The mdBook that is the signer's documentation, built for agents first: `llms.txt` (an index with each chapter's description and size), `llms-full.txt`, and each chapter's Markdown beside the HTML. Readers choose chapters from that index, so a chapter's `description` decides whether it gets read.

## Where to edit

- Chapters live in `docs/src/`: using the signer at the top level, working on it in `docs/src/dev/` (contributing, code map, plan, changelog, git-review), and apps in `docs/src/apps/`. A chapter that is only frontmatter plus `{{#include ...}}` of a repository file (security, and contributing, plan, changelog and git-review in `dev/`) is written in that file: edit the file, never the wrapper or anything in `target/`. An include path is relative to the chapter, so it gains a `../` per folder level.
- `README.md` at the repository root is a short front page linking to the published book, not a chapter.
- Apps under `apps/` are documented in `docs/src/apps/`, listed under `# Extras` at the bottom of `SUMMARY.md`: a chapter per app, plus a row in `docs/src/apps/README.md`.

## Writing a chapter

- Start with frontmatter: `description: "..."` in one line, naming what the chapter covers in the reader's terms. It becomes the chapter's line in `llms.txt`.
- Add it to `SUMMARY.md` in the right section; an unlisted file is not in the book.
- One physical line per paragraph, list item and table row; never hard-wrap prose. No em dashes; use a colon, a comma or a hyphen.
- Plain and exact: say what the signer does and refuses, with the field, flag, exit code or path. Say what is not covered rather than implying it is. State facts once and link to the chapter that owns them instead of repeating them.
- Links are relative paths to files in the repository (`../setup.md#isolation`), never into `target/` and never absolute site URLs. Links inside an included repository file resolve from that file, so `SECURITY.md` links to the published site instead.
- Mermaid diagrams go in fenced `mermaid` blocks; they render in the HTML and stay as source in the llms output.
- A change to behaviour, a flag, a config key or a route updates its chapter in the same change. Security-relevant gaps go in `SECURITY.md` (threat model or "Not covered"), and planned work in `PLAN.md`.

## Checking

Run `just docs check` from the repository root (or `just check` inside `docs/`) after any doc change: it fails on a build warning, an empty chapter (a missing include) or a link that leads nowhere. `just docs` builds the book and `just docs serve` previews it; `just docs setup` installs mdBook, mdbook-mermaid and mdbook-llms once.

Inside `docs/`, `just check` is this book check, not the signer's gate. Run the gate from the repository root.

Docs-only changes use `docs:` commits, which do not bump the signer's version.
