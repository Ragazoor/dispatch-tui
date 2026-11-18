#!/usr/bin/env bash
# test-verify-level.sh — behavioural test for scripts/verify-level.sh.
#
# verify.sh runs only as much as the branch changed (mcp-task-tools.allium:
# AgentsRunTheVerifyCommand). verify-level.sh makes that choice. These cases pin it:
#   - nothing changed (no commits ahead of the base, no edits, no untracked
#     files) prints "none";
#   - only paths no test or gate reads as code changed prints "gates";
#   - anything else prints "full", and so does a base with no merge-base.
# Uncommitted and untracked files count, because verify runs before the commit.
#
# Hermetic: every case runs in a temp git repository.
#
# Run from the repo root:  bash scripts/test-verify-level.sh
set -euo pipefail

# Inherited from a git hook, these would point the fixture's git commands at
# the real repository (see gates.sh).
unset GIT_DIR GIT_INDEX_FILE GIT_WORK_TREE

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LEVEL="$SCRIPT_DIR/verify-level.sh"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

# A fresh repository on main with one commit, and a branch "work" from it.
fresh() {
    rm -rf "$WORKDIR/repo"
    mkdir -p "$WORKDIR/repo/docs/specs" "$WORKDIR/repo/docs/guide" "$WORKDIR/repo/docs/plans" "$WORKDIR/repo/src"
    cd "$WORKDIR/repo"
    git init -q -b main
    git config user.email test@example.com
    git config user.name test
    git config commit.gpgsign false
    echo a >docs/conventions.md
    echo a >docs/specs/core.allium
    echo a >docs/guide/index.html
    echo a >src/lib.rs
    echo a >README.md
    echo a >CLAUDE.md
    git add -A
    git commit -q -m init
    git checkout -q -b work
}

expect() {
    local want="$1" label="$2"
    shift 2
    local got
    got="$(bash "$LEVEL" "$@")" || fail "$label: verify-level.sh exited non-zero"
    [ "$got" = "$want" ] || fail "$label: want '$want', got '$got'"
}

commit_all() {
    git add -A
    git commit -q -m change
}

fresh
expect none "no change"

fresh
echo b >docs/plans/plan.md
commit_all
expect gates "committed plan doc"

fresh
echo b >>docs/conventions.md
expect gates "uncommitted edit to a doc"

fresh
echo b >docs/research.md
expect gates "untracked doc"

fresh
echo b >>README.md
commit_all
expect gates "README"

fresh
echo b >>docs/specs/core.allium
commit_all
expect full "spec: tests read docs/specs"

fresh
echo b >>docs/guide/index.html
commit_all
expect full "guide: a test reads docs/guide"

fresh
echo b >>CLAUDE.md
commit_all
expect full "CLAUDE.md: a test reads it"

fresh
echo b >>docs/conventions.md
echo b >>src/lib.rs
commit_all
expect full "doc and code"

fresh
echo b >src/new.rs
expect full "untracked code"

fresh
git rm -q docs/conventions.md
commit_all
expect gates "deleted doc"

fresh
git rm -q src/lib.rs
commit_all
expect full "deleted code"

fresh
git mv -f src/lib.rs docs/moved.md
commit_all
expect full "code moved into docs: both paths count"

fresh
echo b >docs/plans/plan.md
commit_all
git checkout -q main
echo c >>src/lib.rs
commit_all
git checkout -q work
expect gates "main moved: only the branch's own change counts"

fresh
git checkout -q main
expect none "explicit base" main
git checkout -q work

fresh
git branch -q other
echo b >docs/plans/plan.md
commit_all
expect gates "base passed as an argument" other

fresh
expect full "unknown base: no merge-base" no-such-branch

echo "verify-level self-test: ok"
