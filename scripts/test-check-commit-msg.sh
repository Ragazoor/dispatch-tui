#!/usr/bin/env bash
# test-check-commit-msg.sh — behavioural test for scripts/check-commit-msg.sh.
#
# Pins the subjects the checker must accept (every type, scope, `!`, the
# exempt merge/revert/autosquash forms, comment lines before the subject) and
# the ones it must reject (no type, unknown type, missing space or description,
# empty message).
#
# Run from the repo root:  bash scripts/test-check-commit-msg.sh
# Exits 0 on success, non-zero with a diagnostic on the first failed assertion.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECKER="$SCRIPT_DIR/check-commit-msg.sh"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

failures=0

# $1 is the expected exit status, $2 the message body, $3 the assertion label.
expect() {
    local want="$1" body="$2" label="$3"
    printf '%s\n' "$body" >"$WORKDIR/msg"
    local got=0
    bash "$CHECKER" "$WORKDIR/msg" >/dev/null 2>&1 || got=$?
    if [ "$got" -ne "$want" ]; then
        echo "FAIL: $label (wanted exit $want, got $got)" >&2
        failures=$((failures + 1))
    fi
}

for type in feat fix docs chore refactor test perf ci build style revert; do
    expect 0 "$type: do the thing" "accepts type $type"
done
expect 0 "feat(tui): add a pane" "accepts a scope"
expect 0 "fix(spacetime-module)!: drop a column" "accepts a hyphenated scope and !"
expect 0 "refactor!: rename the alias" "accepts ! without a scope"
expect 0 $'fix: a subject\n\nBody text.\n\nBREAKING CHANGE: gone' "accepts a body and footer"
expect 0 $'# Please enter the commit message\nfix: after a comment' "skips comment lines"
expect 0 "Merge branch 'main' into feature" "exempts a merge commit"
expect 0 'Revert "feat: add a pane"' "exempts git's revert subject"
expect 0 "fixup! feat: add a pane" "exempts fixup!"
expect 0 "squash! feat: add a pane" "exempts squash!"

expect 1 "Fix the scroll count" "rejects a plain sentence"
expect 1 "wip" "rejects a bare word"
expect 1 "Feat: capitalised type" "rejects a capitalised type"
expect 1 "feature: unknown type" "rejects an unknown type"
expect 1 "fix:no space" "rejects a missing space after the colon"
expect 1 "fix: " "rejects an empty description"
expect 1 "fix(): empty scope" "rejects an empty scope"
expect 1 "fix(tui: unclosed scope" "rejects an unclosed scope"
expect 1 "" "rejects an empty message"
expect 1 $'# only a comment' "rejects a comment-only message"
expect 1 $'\nfix: blank first line' "rejects a blank subject line"

if [ "$failures" -ne 0 ]; then
    echo "$failures assertion(s) failed" >&2
    exit 1
fi
echo "check-commit-msg self-test passed"
