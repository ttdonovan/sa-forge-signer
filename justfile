set shell := ["bash", "-euo", "pipefail", "-c"]

[private]
default:
    @just --list --unsorted --list-submodules

# Lint gate (same as CI): fmt, clippy (warnings are errors), tests, shellcheck, advisories, dependency age
[group('verify')]
check:
    @scripts/ci/check.sh

# Time the audit log's reads on synthetic logs (release build; writes temporary files, prints a table)
[group('verify')]
bench:
    cargo test --release --locked bench:: -- --ignored --nocapture --test-threads 1

# Release build
[group('build')]
build:
    cargo build --release --locked

# Build the debug binaries that this repository's .mcp.json and `just mcp-dev` start; restart the Claude session afterwards
[group('mcp')]
mcp-build:
    cargo build --locked --bins

# Run the debug service for .mcp.json on a throwaway home in target/dev (127.0.0.1:18790); leave it running
[group('mcp')]
mcp-dev:
    @scripts/dev/mcp-dev.sh

# The docs book (`just docs`, `just docs check`) and the apps/ extras (`just apps <recipe>`).

mod docs
mod apps

# Local git review (review, diff, approve, ...), self-contained in scripts/git-review/.

import "scripts/git-review/justfile"
