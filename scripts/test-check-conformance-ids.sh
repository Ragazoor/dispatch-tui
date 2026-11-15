#!/usr/bin/env bash
# test-check-conformance-ids.sh — behavioural test for
# scripts/check-conformance-ids.sh. Hermetic: each case runs the checker against
# a fixture repo in a temp dir.
#
# Run from the repo root:  bash scripts/test-check-conformance-ids.sh
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECKER="$SCRIPT_DIR/check-conformance-ids.sh"
WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

failures=0

# $1 expected exit status, $2 file body (placed in the conformance dir), $3 label.
expect() {
    local want="$1" body="$2" label="$3"
    rm -rf "${WORKDIR:?}/tests"
    mkdir -p "$WORKDIR/tests/memory_caller_conformance"
    printf '%s\n' "$body" >"$WORKDIR/tests/memory_caller_conformance/case.rs"
    local got=0 out
    out="$(cd "$WORKDIR" && bash "$CHECKER" 2>&1)" || got=$?
    if [[ "$got" != "$want" ]]; then
        echo "FAIL: $label (expected exit $want, got $got)" >&2
        echo "  output: $out" >&2
        failures=$((failures + 1))
    fi
}

expect 1 'assert_eq!(r, m, "both sides hand out the same task id");' \
    "the old create-closure assertion is rejected"
expect 1 'assert_eq!(a_real, a_mem, "generated epic id");' \
    "a generated-id message is rejected"
expect 1 'assert_eq!(
    epic_scoped_real, epic_scoped_mem,
    "generated epic-scoped learning id"
);' "a rustfmt-wrapped assertion is rejected"
expect 0 'assert_eq!(r, m, "both sides must refuse in the same words");' \
    "an unrelated 'same' message passes"
expect 0 'assert_eq!(shape(&rows_real), shape(&rows_mem), "task shape");' \
    "a shape comparison passes"
expect 0 'assert_eq!(r, m /* allow-cross-store-id: ids are seeded equal */, "same task id");' \
    "an annotated assertion passes"
expect 0 'let x = 1; // the same id, in a comment' \
    "prose outside an assertion passes"

if [[ "$failures" != 0 ]]; then
    echo "$failures case(s) failed" >&2
    exit 1
fi
echo "check-conformance-ids self-test: ok"
