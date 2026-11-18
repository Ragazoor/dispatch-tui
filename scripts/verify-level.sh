#!/bin/sh
# verify-level.sh — how much the verify command must check: prints one of
#   none   nothing changed against the base: no commits ahead of it, no
#          uncommitted edits, no untracked files;
#   gates  only paths no test or gate reads as code changed: run the gates,
#          not the tests;
#   full   anything else, or no merge-base with the base: run everything.
#
# The change is the working tree against the merge-base with the base, plus
# untracked files, because verify runs before the commit. Commits that landed
# on the base since the branch forked do not count (mcp-task-tools.allium:
# AgentsRunTheVerifyCommand).
#
# Usage:  ./scripts/verify-level.sh [base]   (base defaults to main)
# Self-test: scripts/test-verify-level.sh
set -eu

base="${1:-main}"

if ! mb="$(git merge-base HEAD "$base" 2>/dev/null)"; then
    echo full
    exit 0
fi

# A path is safe when no test reads it and the gates check it themselves.
# Tests read docs/specs/ (the spec list), docs/guide/ (the key binding guide)
# and CLAUDE.md, so those are code here. A *.allium file anywhere under docs/
# decides the dispatch prompt's design step (src/dispatch/allium_specs.rs), so
# it is code too. Anything not listed is code.
is_safe() {
    case "$1" in
        docs/specs/* | docs/guide/* | *.allium) return 1 ;;
        docs/* | README.md) return 0 ;;
        *) return 1 ;;
    esac
}

changed="$(
    git diff --name-only --no-renames "$mb"
    git ls-files --others --exclude-standard
)"

if [ -z "$changed" ]; then
    echo none
    exit 0
fi

level=gates
# Paths come one per line; a name with a newline in it is code.
echo "$changed" | while IFS= read -r path; do
    is_safe "$path" || exit 1
done || level=full
echo "$level"
