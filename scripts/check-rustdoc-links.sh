#!/usr/bin/env bash
# Fail when a rustdoc intra-doc link in src/ names nothing. check-doc-symbols.sh
# reads the same doc comments for bare names; this is the compiler's own answer
# for `[`Item`]` links, which that script cannot resolve.
#
# Private items are documented too (--document-private-items): most of the
# repo's links point at private helpers, and a stale one is just as stale.
#
# src/spacetime/bindings/ is generated (scripts/regenerate-spacetime-bindings.sh)
# and its doc comments carry links the compiler cannot resolve. They are not
# editable, so their findings are dropped, as check-doc-symbols.sh does.
#
# Run from the repo root. Pass a log file to check it instead of running rustdoc
# (the self-test does this).
set -euo pipefail

if [[ $# -gt 0 ]]; then
    LOG="$1"
else
    LOG="$(mktemp)"
    trap 'rm -f "$LOG"' EXIT
    cargo doc --no-deps --document-private-items >"$LOG" 2>&1 || true
fi

# `unresolved link` is the finding; its location is the next `-->` line.
found="$(awk '
    /^(warning|error): unresolved link/ { msg = $0; want = 1; next }
    want && /-->/ { print $2 "\t" msg; want = 0 }
' "$LOG" | grep -v '^src/spacetime/bindings/' || true)"

if [[ -n "$found" ]]; then
    echo "check-rustdoc-links: unresolved rustdoc link(s):" >&2
    while IFS=$'\t' read -r loc msg; do
        echo "  $loc  ${msg#*: }" >&2
    done <<<"$found"
    exit 1
fi

echo "check-rustdoc-links: all rustdoc links resolve"
