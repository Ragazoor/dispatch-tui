#!/bin/sh
# gates.sh — the lint and check gates, shared by the pre-push hook and
# verify.sh (this directory) so the two cannot drift.
#
# A rebase-mode wrap-up lands commits on `main` without a push, so the hook
# never sees them. The verify command runs this script, which holds a commit to
# the same gates a push meets. (Clippy failed on committed `main` this way on
# 2026-11-04; two checkers did on 2026-10-14.)
#
# Usage:  ./scripts/gates.sh [--check | --fix]
#   --check (default)  `cargo fmt --check`: fail on unformatted code.
#   --fix              `cargo fmt`: reformat in place. The pre-push hook uses it.
#
# The toolchain check is a hard failure, not a skip: a distro cargo without
# rustup ignores rust-toolchain.toml, and its clippy disagrees with CI. See
# docs/testing.md, "Toolchain and pre-push hook".
#
# `check-commit-msg.sh` takes a message file, so it is not run here; the
# commit-msg hook runs it and its self-test runs below.
#
# Run from the repo root.
set -e

fmt_mode="${1:---check}"
case "$fmt_mode" in
    --check | --fix) ;;
    *)
        echo "usage: gates.sh [--check | --fix]" >&2
        exit 2
        ;;
esac

# First: a host cargo that ignores rust-toolchain.toml makes fmt and clippy
# disagree with CI, so say so before they report errors the change did not cause.
echo "toolchain..."
./scripts/check-toolchain.sh

echo "toolchain self-test..."
bash ./scripts/test-check-toolchain.sh

echo "fmt..."
if [ "$fmt_mode" = "--fix" ]; then
    cargo fmt
else
    cargo fmt --check
fi

echo "clippy..."
cargo clippy --all-targets -- -D warnings

echo "doc-paths..."
./scripts/check-doc-paths.sh

echo "doc-paths self-test..."
bash ./scripts/test-check-doc-paths.sh

echo "doc-symbols..."
./scripts/check-doc-symbols.sh

echo "doc-symbols self-test..."
bash ./scripts/test-check-doc-symbols.sh

echo "doc-headings..."
./scripts/check-doc-headings.sh

echo "doc-headings self-test..."
bash ./scripts/test-check-doc-headings.sh

echo "rustdoc-links self-test..."
bash ./scripts/test-check-rustdoc-links.sh

echo "rustdoc-links..."
./scripts/check-rustdoc-links.sh

echo "no-test-sleep..."
./scripts/check-no-test-sleep.sh

echo "no-test-sleep self-test..."
bash ./scripts/test-check-no-test-sleep.sh

echo "single-write-service..."
./scripts/check-single-write-service.sh

echo "single-write-service self-test..."
bash ./scripts/test-check-single-write-service.sh

echo "commit-msg self-test..."
bash ./scripts/test-check-commit-msg.sh

echo "fetch-reviews..."
bash ./scripts/test-fetch-reviews.sh

echo "fetch-cve..."
bash ./scripts/test-fetch-cve.sh

echo "bots.conf validation..."
bash ./scripts/test-bots-conf.sh

# The module the board embeds and publishes must match spacetime/module's
# source. Cheap (hashes files, builds nothing), so it runs before the slow one.
echo "embedded module stamp..."
bash ./scripts/build-managed-module.sh --check

# The upgrade smoke test's committed store must hold a module main shipped.
# Cheap: hashes committed blobs, builds nothing.
echo "previous-store fixture..."
./scripts/check-previous-store.sh

echo "previous-store self-test..."
bash ./scripts/test-check-previous-store.sh

# Last: it builds a second crate for a second target, so it is the slowest step
# and the one most worth reaching only after the cheap checks have spoken.
echo "spacetime module..."
bash ./scripts/check-spacetime-module.sh
