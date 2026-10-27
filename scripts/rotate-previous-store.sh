#!/usr/bin/env bash
# Move tests/fixtures/previous-store/ to the module local `main` runs now.
#
#   scripts/rotate-previous-store.sh [fixture-dir]
#
# Run it after a module change lands on `main` (scripts/check-previous-store.sh
# says when). It is itself an upgrade: it starts the committed store, publishes
# main's src/spacetime/module.wasm over it with --delete-data=never, and keeps
# the result. A module main cannot migrate to fails here the way it would for
# an operator.
#
# expected.json is left alone: the rows must survive an upgrade. The upgrade
# smoke test (src/runtime/tests/startup_smoke.rs) checks that they do.
#
# Uses local `main`, never `origin/main`, and never the working tree's module,
# so a module rebuilt twice on a branch never becomes "previous".
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
FIXTURE="$(realpath "${1:-tests/fixtures/previous-store}")"
WORK="$(mktemp -d)"
PID=""
cleanup() {
    if [ -n "$PID" ]; then kill "$PID" 2>/dev/null || true; fi
    rm -rf "$WORK"
}
trap cleanup EXIT

if ! git rev-parse --verify -q main >/dev/null; then
    echo "error: no local main to take the module from" >&2
    exit 1
fi
git show main:src/spacetime/module.wasm >"$WORK/main.wasm"
HASH="sha256:$(sha256sum "$WORK/main.wasm" | cut -d' ' -f1)"
COMMIT="$(git rev-parse --short=8 main)"

cp -r "$FIXTURE" "$WORK/store"
CONFIG="$WORK/store/cli.toml"
PORT="$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"
URL="http://127.0.0.1:$PORT"

echo "previous-store: starting the fixture's store on $URL"
spacetime --config-path="$CONFIG" start \
    --listen-addr="127.0.0.1:$PORT" \
    --data-dir="$WORK/store/data" \
    --jwt-priv-key-path="$WORK/store/keys/id_ecdsa" \
    --jwt-pub-key-path="$WORK/store/keys/id_ecdsa.pub" \
    --non-interactive >"$WORK/store.log" 2>&1 &
PID=$!

for _ in $(seq 1 300); do
    if curl -s -o /dev/null "$URL/v1/identity"; then break; fi
    if ! kill -0 "$PID" 2>/dev/null; then
        echo "error: the store exited during startup:" >&2
        cat "$WORK/store.log" >&2
        exit 1
    fi
    sleep 0.1
done

echo "previous-store: publishing main's module ($COMMIT, $HASH)"
spacetime --config-path="$CONFIG" publish -b "$WORK/main.wasm" -s "$URL" -y \
    --delete-data=never dispatch

echo "$HASH" >"$WORK/store/data/managed-module.sha256"

# SIGTERM, then wait: the store flushes before its directory is copied.
kill -TERM "$PID"
wait "$PID" 2>/dev/null || true
PID=""

rm -rf "$WORK/store/data/cache" "$WORK/store/data/logs" \
    "$WORK/store/data/spacetime.pid" "$WORK/store/data/dispatch_module.wasm"
rm -rf "$FIXTURE/data"
cp -r "$WORK/store/data" "$FIXTURE/data"

SPACETIME_VERSION="$(spacetime --version 2>/dev/null | sed -n 's/^spacetimedb tool version \([^;]*\);.*/\1/p')"
sed -i \
    -e "s/^- main commit: .*/- main commit: $COMMIT/" \
    -e "s/^- spacetime version: .*/- spacetime version: $SPACETIME_VERSION/" \
    -e "s/^- module hash: .*/- module hash: $HASH/" \
    "$FIXTURE/README.md"

echo "previous-store: now on $COMMIT; run the upgrade smoke test, then commit $FIXTURE"
