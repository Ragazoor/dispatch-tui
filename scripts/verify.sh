#!/bin/sh
# verify.sh — the repo's verify command: as much of the pre-push gates and the
# test suite as the branch's change needs (scripts/verify-level.sh decides):
#   none   nothing changed against the base: nothing runs;
#   gates  only docs no test reads changed: the gates, in check mode;
#   full   anything else: the gates, then the full test suite.
# It prints the level first, so the agent can say what ran.
#
# Whenever it runs the gates (every level but none), it fails when the host
# rustc is not the pinned version (see gates.sh, check-toolchain.sh): rustup
# keeps them equal, a distro rustc only by luck.
# Never reformats the tree.
#
# Run from the repo root:  ./scripts/verify.sh [base]   (base defaults to main)
set -e

base="${1:-main}"
level="$(./scripts/verify-level.sh "$base")"

case "$level" in
    none)
        echo "verify: no change against $base, nothing to check"
        exit 0
        ;;
    gates) echo "verify: only docs no test reads changed against $base, gates only (tests skipped)" ;;
    *) echo "verify: code changed against $base, gates and the full test suite" ;;
esac

./scripts/gates.sh --check

if [ "$level" = full ]; then
    echo "tests..."
    cargo test --no-fail-fast
fi
