# sa-forge-signer

The docs are a book for agents. Run `just docs` (install the tools once with `just docs-setup`), then read `target/docs/html/llms.txt` first: an index of every chapter with a line on each. Read only the chapters a task needs (each `.md` beside it), or `llms-full.txt` for the whole book.

Most chapters are written in `docs/src/`; a few include a file from the repository (`SECURITY.md`, `CONTRIBUTING.md`, `PLAN.md`, `CHANGELOG.md`, ...): change those files, not their chapters or the copies in `target/`. `README.md` is a short front page linking to the published book, not a chapter. Links point at files in the repository, never into `target/`. A new doc gets a chapter in `docs/src/` and a line in `docs/src/SUMMARY.md`; run `just docs-check` after changing any doc.

`just check` is the gate, the same as CI. Commit messages use Conventional Commits (`feat:`, `fix:`, `!` for a breaking change) and end with a full stop: release-plz reads them to bump the version, so see `CONTRIBUTING.md` for what counts as breaking.
