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

# The docs book and the apps/ extras.

import "docs/justfile"
import "apps/justfile"

# Local git review (review, diff, approve, ...), self-contained in scripts/git-review/.

import "scripts/git-review/justfile"
