#!/usr/bin/env bash
# test-check-allium.sh — behavioural test for scripts/check-allium.sh.
#
# Hermetic: each case runs the checker in a temp fixture repo (two empty specs
# and a baseline) against a stub `allium` that prints canned JSON. Checking this
# repo's own specs with the real CLI is check-allium.sh's own job, run as its
# own gate step.
#
# Run from the repo root:  bash scripts/test-check-allium.sh
# Exits 0 on success, non-zero with a diagnostic on the first failed assertion.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECKER="$SCRIPT_DIR/check-allium.sh"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

REPO="$WORKDIR/repo"
STUB_BIN="$WORKDIR/bin"
mkdir -p "$REPO/docs/specs" "$REPO/scripts" "$STUB_BIN"
: >"$REPO/docs/specs/a.allium"
: >"$REPO/docs/specs/b.allium"

# The stub answers `--version` with $STUB_VERSION and `check` by printing
# $STUB_OUT and exiting $STUB_RC.
cat >"$STUB_BIN/allium" <<'STUB'
#!/usr/bin/env bash
if [ "$1" = "--version" ]; then
    echo "allium $STUB_VERSION (language versions: 1, 2, 3)"
    exit 0
fi
cat "$STUB_OUT"
exit "$STUB_RC"
STUB
chmod +x "$STUB_BIN/allium"

# jq, awk, sort and the shell come from the system; a real `allium` (usually in
# ~/.cargo/bin) must not, or the "missing" case could not be tested.
SYS_PATH="/usr/local/bin:/usr/bin:/bin"

failures=0

# One spec's JSON document. $1 the spec file, $2 its diagnostics array.
doc() {
    printf '{"command":"check","diagnostics":%s,"findings":[],"spec_file":"%s"}\n' "$2" "$1"
}

# One diagnostic. $1 severity, $2 file, $3 line, $4 code, $5 message.
diag() {
    printf '{"code":"%s","location":{"col":1,"file":"%s","line":%s},"message":"%s","severity":"%s"}' \
        "$4" "$2" "$3" "$5" "$1"
}

BASELINED="$(diag warning docs/specs/a.allium 10 allium.definition.unused "Value 'Known' is declared but not referenced elsewhere.")"
MOVED="$(diag warning docs/specs/a.allium 99 allium.definition.unused "Value 'Known' is declared but not referenced elsewhere.")"
UNLISTED="$(diag warning docs/specs/b.allium 5 allium.definition.unused "Value 'New' is declared but not referenced elsewhere.")"
ERROR="$(diag error docs/specs/b.allium 7 allium.rule.undefinedTypeReference "Type reference 'Nope' is not declared locally or imported.")"
INFO="$(diag info docs/specs/b.allium 3 allium.field.unused "Field 'x' is declared but not used.")"

printf '%s\n' \
    '# a comment' \
    '' \
    "docs/specs/a.allium	allium.definition.unused	Value 'Known' is declared but not referenced elsewhere." \
    >"$REPO/scripts/allium-check.baseline"

# $1 expected exit, $2 label, $3 the stub's stdout, $4 its exit (default 1),
# $5 its version (default 3.6.1), $6 "nostub" to hide the stub. Optional $7: a
# string the combined output must contain.
expect() {
    local want="$1" label="$2" body="$3" rc="${4:-1}" version="${5:-3.6.1}" mode="${6:-}" needle="${7:-}"
    printf '%s' "$body" >"$WORKDIR/out.json"

    local path="$STUB_BIN:$SYS_PATH"
    [ "$mode" = nostub ] && path="$SYS_PATH"

    local got=0 out
    out="$(cd "$REPO" && PATH="$path" STUB_OUT="$WORKDIR/out.json" STUB_RC="$rc" \
        STUB_VERSION="$version" bash "$CHECKER" 2>&1)" || got=$?

    if [ "$got" != "$want" ]; then
        echo "FAIL: $label" >&2
        echo "  expected exit $want, got $got" >&2
        echo "  output: $out" >&2
        failures=$((failures + 1))
    elif [ -n "$needle" ] && ! printf '%s' "$out" | grep -qF -- "$needle"; then
        echo "FAIL: $label" >&2
        echo "  output lacks: $needle" >&2
        echo "  output: $out" >&2
        failures=$((failures + 1))
    fi
}

# --- What passes. ------------------------------------------------------------

expect 0 'no diagnostics passes' \
    "$(doc docs/specs/a.allium '[]')$(doc docs/specs/b.allium '[]')" 0

expect 0 'info diagnostics alone pass and are counted' \
    "$(doc docs/specs/a.allium '[]')$(doc docs/specs/b.allium "[$INFO]")" 0 3.6.1 '' '1 info'

expect 0 'a baselined warning passes' \
    "$(doc docs/specs/a.allium "[$BASELINED]")$(doc docs/specs/b.allium '[]')"

expect 0 'a baselined warning on another line still passes' \
    "$(doc docs/specs/a.allium "[$MOVED]")$(doc docs/specs/b.allium '[]')"

expect 0 'a baseline entry the checker no longer reports is a note, not a failure' \
    "$(doc docs/specs/a.allium '[]')$(doc docs/specs/b.allium '[]')" 0 3.6.1 '' 'no longer reports'

expect 0 'a newer CLI passes the version floor (numeric, not string, compare)' \
    "$(doc docs/specs/a.allium '[]')$(doc docs/specs/b.allium '[]')" 0 3.10.0

# --- What fails. -------------------------------------------------------------

expect 1 'an error fails' \
    "$(doc docs/specs/a.allium '[]')$(doc docs/specs/b.allium "[$ERROR]")" 1 3.6.1 '' "Nope"

expect 1 'an error fails even beside a baselined warning' \
    "$(doc docs/specs/a.allium "[$BASELINED]")$(doc docs/specs/b.allium "[$ERROR]")"

expect 1 'an unbaselined warning fails and prints the line to add' \
    "$(doc docs/specs/a.allium '[]')$(doc docs/specs/b.allium "[$UNLISTED]")" 1 3.6.1 '' \
    "docs/specs/b.allium	allium.definition.unused	Value 'New' is declared"

expect 1 'a baselined warning in another file does not cover this one' \
    "$(doc docs/specs/a.allium '[]')$(doc docs/specs/b.allium "[$(diag warning docs/specs/b.allium 10 allium.definition.unused "Value 'Known' is declared but not referenced elsewhere.")]")"

expect 1 'a missing allium CLI fails' \
    "" 0 3.6.1 nostub 'cargo install allium-cli'

expect 1 'an allium CLI older than the floor fails' \
    "$(doc docs/specs/a.allium '[]')$(doc docs/specs/b.allium '[]')" 0 3.6.0 '' 'older than'

expect 1 'exit 2 (no inputs) fails' \
    "" 2

expect 1 'empty output fails rather than passing as clean' \
    "" 0

expect 1 'output not JSON fails' \
    "panic: something broke" 1

expect 1 'a report covering fewer specs than were passed fails' \
    "$(doc docs/specs/a.allium '[]')" 0 3.6.1 '' 'reported on 1 of 2'

if [ "$failures" -ne 0 ]; then
    echo "test-check-allium: $failures assertion(s) failed" >&2
    exit 1
fi
echo "test-check-allium: all assertions passed"
