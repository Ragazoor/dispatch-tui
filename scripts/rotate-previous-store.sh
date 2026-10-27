#!/usr/bin/env bash
# Rebuild tests/fixtures/previous-store/ from the module local `main` runs now.
#
#   scripts/rotate-previous-store.sh
#
# Run it after a module change lands on `main` (scripts/check-previous-store.sh
# says when), on a checkout whose src/spacetime/module.wasm is main's — main
# itself, or a branch that has merged main and not changed the module. It
# refuses otherwise: the fixture must hold a module main shipped.
#
# It rebuilds the store from scratch with the generator in
# src/runtime/tests/startup_smoke.rs (fresh keys, logins and rows), then
# fills in the README's three fields. It does not publish over the old store:
# every publish appends the module to the store's commit log, so a fixture
# moved forward that way grows by about 1.6 MB each time. The upgrade from
# the previous module is still tested, by the upgrade smoke test, on every
# branch that changes the module.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
FIXTURE="tests/fixtures/previous-store"
MODULE="src/spacetime/module.wasm"

if ! git rev-parse --verify -q main >/dev/null; then
    echo "error: no local main to take the module from" >&2
    exit 1
fi
MAIN_HASH="sha256:$(git show "main:$MODULE" | sha256sum | cut -d' ' -f1)"
HERE_HASH="sha256:$(sha256sum "$MODULE" | cut -d' ' -f1)"
if [ "$MAIN_HASH" != "$HERE_HASH" ]; then
    cat >&2 <<EOF
error: this checkout's $MODULE is not main's.
  main: $MAIN_HASH
  here: $HERE_HASH
Rebuild the fixture on main, or on a branch that merged main without changing
the module.
EOF
    exit 1
fi

echo "previous-store: rebuilding from main's module ($MAIN_HASH)"
cargo test --lib -- --ignored --exact \
    runtime::tests::startup_smoke::generate_previous_store_fixture

RECORDED="$(tr -d '[:space:]' <"$FIXTURE/data/managed-module.sha256")"
if [ "$RECORDED" != "$MAIN_HASH" ]; then
    echo "error: the rebuilt store records $RECORDED, not main's $MAIN_HASH" >&2
    exit 1
fi

COMMIT="$(git rev-parse --short=8 main)"
SPACETIME_VERSION="$(spacetime --version 2>/dev/null | sed -n 's/^spacetimedb tool version \([^;]*\);.*/\1/p')"
sed -i \
    -e "s/^- main commit: .*/- main commit: $COMMIT/" \
    -e "s/^- spacetime version: .*/- spacetime version: $SPACETIME_VERSION/" \
    -e "s/^- module hash: .*/- module hash: $MAIN_HASH/" \
    "$FIXTURE/README.md"

echo "previous-store: rebuilt on main $COMMIT, $(du -sh "$FIXTURE" | cut -f1)"
echo "Run the upgrade smoke test, then commit $FIXTURE."
