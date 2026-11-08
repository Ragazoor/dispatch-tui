#!/bin/sh
# verify.sh — the repo's verify command: every gate the pre-push hook runs,
# in check mode, then the full test suite.
#
# Fails when the host rustc is not the pinned version (see gates.sh,
# check-toolchain.sh): rustup keeps them equal, a distro rustc only by luck.
# Never reformats the tree.
#
# Run from the repo root:  ./scripts/verify.sh
set -e

./scripts/gates.sh --check

echo "tests..."
cargo test --no-fail-fast
