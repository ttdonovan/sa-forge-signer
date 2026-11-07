set shell := ["bash", "-euo", "pipefail", "-c"]

[private]
default:
    @just --list --unsorted

# Lint gate (same as CI): fmt, clippy (warnings are errors), tests, shellcheck, advisories, dependency age
[group('verify')]
check:
    @scripts/ci/check.sh

# Release build
[group('build')]
build:
    cargo build --release --locked

# Build the docs book (HTML, llms.txt, llms-full.txt, per-chapter Markdown) into target/docs/html/
[group('docs')]
docs:
    @mdbook build docs
    @echo "Agents: read target/docs/html/llms.txt first."

# Serve the docs book with live reload and open it in a browser
[group('docs')]
docs-serve:
    mdbook serve docs --open

# Build the book and fail on an error, a warning, an empty chapter, or a link that leads nowhere
[group('docs')]
docs-check:
    @scripts/ci/docs-check.sh

# Install the book's tools: mdBook 0.5, mdbook-mermaid and mdbook-llms (with mdbook-strip-frontmatter) from GitHub
[group('docs')]
docs-setup:
    cargo install --locked mdbook --version 0.5.2
    cargo install --locked mdbook-mermaid --version 0.17.1
    cargo install --locked --git https://github.com/ttdonovan/mdbook-llms --tag v0.4.0

# Local git review (review, diff, approve, ...), self-contained in scripts/git-review/.

import "scripts/git-review/justfile"
