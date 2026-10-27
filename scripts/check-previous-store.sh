#!/usr/bin/env bash
# Check that tests/fixtures/previous-store/ was written by a module `main` ran.
#
# The upgrade smoke test (src/runtime/tests/startup_smoke.rs) starts that
# store and runs this branch's startup over it. It only means "an upgrade from
# what users run" if the store's module is one `main` actually shipped: one
# built from an unmerged branch would test an upgrade from nowhere.
#
# A fixture on an OLDER `main` module passes, with a note: it still tests a
# real upgrade, only from further back, and failing every branch's push the
# moment a module change merges would block everyone until someone rotates.
# Rotate with scripts/rotate-previous-store.sh.
#
# Compares against local `main` (never `origin/main`). Without one — a shallow
# CI checkout — it skips, saying so.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
RECORD="tests/fixtures/previous-store/data/managed-module.sha256"
MODULE="src/spacetime/module.wasm"

if [ ! -f "$RECORD" ]; then
    echo "error: $RECORD is missing: no module hash record for the previous store" >&2
    exit 1
fi
fixture="$(tr -d '[:space:]' <"$RECORD")"

if ! git rev-parse --verify -q main >/dev/null; then
    echo "previous-store: skipping: no local main to compare against"
    exit 0
fi

hash_at() {
    echo "sha256:$(git show "$1:$MODULE" | sha256sum | cut -d' ' -f1)"
}

if [ "$fixture" = "$(hash_at main)" ]; then
    echo "previous-store: on main's current module"
    exit 0
fi

for commit in $(git log --format=%H main -- "$MODULE"); do
    if git cat-file -e "$commit:$MODULE" 2>/dev/null && [ "$fixture" = "$(hash_at "$commit")" ]; then
        echo "previous-store: on main's module from ${commit:0:8}, not its current one;"
        echo "  run scripts/rotate-previous-store.sh to test upgrades from what main runs now"
        exit 0
    fi
done

cat >&2 <<EOF
error: tests/fixtures/previous-store/ holds module $fixture,
which no commit on main ever had as $MODULE. The upgrade smoke test would
upgrade from a module nobody ran. Rebuild it from main with
scripts/rotate-previous-store.sh.
EOF
exit 1
