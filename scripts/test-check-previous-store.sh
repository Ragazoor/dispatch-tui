#!/usr/bin/env bash
# test-check-previous-store.sh — behavioural test for
# scripts/check-previous-store.sh.
#
# Each case builds a throwaway git repo whose `main` has committed two
# versions of src/spacetime/module.wasm, points the fixture's hash record at
# one of them (or at neither), and runs the checker there.
#
# Run from the repo root:  bash scripts/test-check-previous-store.sh
# Exits 0 on success, non-zero with a diagnostic on the first failed assertion.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECKER="$SCRIPT_DIR/check-previous-store.sh"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT
REPO="$WORKDIR/repo"
RECORD="tests/fixtures/previous-store/data/managed-module.sha256"

git init -q -b main "$REPO"
mkdir -p "$REPO/src/spacetime" "$REPO/$(dirname "$RECORD")"
(
    cd "$REPO"
    git config user.email test@example.com
    git config user.name test
    printf 'old module' >src/spacetime/module.wasm
    git add . && git commit -qm old
    printf 'new module' >src/spacetime/module.wasm
    git add . && git commit -qm new
)
OLD="sha256:$(printf 'old module' | sha256sum | cut -d' ' -f1)"
NEW="sha256:$(printf 'new module' | sha256sum | cut -d' ' -f1)"

failures=0

# $1 expected exit status, $2 the record's content ("" for no record),
# $3 a string the output must contain, $4 the label. $5, when "nomain", runs
# in a checkout with no local main.
expect() {
    local want="$1" record="$2" needle="$3" label="$4" where="${5:-}"
    rm -f "$REPO/$RECORD"
    [ -n "$record" ] && printf '%s\n' "$record" >"$REPO/$RECORD"
    local dir="$REPO"
    if [ "$where" = shallow ]; then
        # CI on a push to main: a local main, but only its tip.
        rm -rf "$WORKDIR/clone"
        git clone -q --depth 1 "file://$REPO" "$WORKDIR/clone"
        mkdir -p "$WORKDIR/clone/$(dirname "$RECORD")"
        [ -n "$record" ] && printf '%s\n' "$record" >"$WORKDIR/clone/$RECORD"
        dir="$WORKDIR/clone"
    elif [ "$where" = nomain ]; then
        rm -rf "$WORKDIR/clone"
        git clone -q --no-local "$REPO" "$WORKDIR/clone"
        (cd "$WORKDIR/clone" && git checkout -q --detach && git branch -qD main)
        mkdir -p "$WORKDIR/clone/$(dirname "$RECORD")"
        [ -n "$record" ] && printf '%s\n' "$record" >"$WORKDIR/clone/$RECORD"
        dir="$WORKDIR/clone"
    fi

    local got=0 out
    out="$(cd "$dir" && bash "$CHECKER" 2>&1)" || got=$?
    if [ "$got" -ne "$want" ]; then
        echo "FAIL: $label — expected exit $want, got $got; output:"
        echo "$out"
        failures=$((failures + 1))
    elif ! printf '%s' "$out" | grep -qF -- "$needle"; then
        echo "FAIL: $label — output lacks \"$needle\"; output:"
        echo "$out"
        failures=$((failures + 1))
    else
        echo "ok: $label"
    fi
}

expect 0 "$NEW" "main's current module" 'fixture on main'\''s current module passes'
expect 0 "$OLD" 'rotate-previous-store.sh' 'fixture on an older main module passes, suggesting a rotation'
expect 1 'sha256:0000' 'rotate-previous-store.sh' 'fixture on a module main never had fails'
expect 1 '' 'no module hash record' 'fixture without a hash record fails'
expect 0 "$NEW" 'skipping' 'no local main skips with a reason' nomain
expect 0 "$OLD" 'skipping' 'a shallow main skips: it cannot see older modules' shallow

if [ "$failures" -ne 0 ]; then
    echo "$failures case(s) failed"
    exit 1
fi
