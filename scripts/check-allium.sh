#!/usr/bin/env bash
# check-allium.sh — run `allium check` over every spec and fail on a new problem.
#
# The specs in docs/specs/ are the shared source of truth (CLAUDE.md, "Working
# With the User"), and nothing checked them structurally before this gate: a
# broken `use`, an undefined entity or an unreachable rule could land on `main`.
#
# Fails on:
#   - any diagnostic with severity `error`;
#   - any `warning` not listed in scripts/allium-check.baseline.
# `info` diagnostics (rule.unreachableTrigger, field.unused, ...) never fail;
# their count is printed. `findings` are always empty from `allium check` (they
# belong to `allium analyse`), so they are not read.
#
# The baseline holds the warnings the checker reports wrongly (its known gaps,
# filed on juxt/allium-tools; see docs/plans/2026-11-05-98956-allium-spec-housekeeping.md,
# "Checker facts"). Each one has a comment beside the declaration in the spec
# saying why it is expected. An entry is keyed on file, code and message, not
# on the line, so an edit elsewhere in the file does not break it. A baseline
# entry the checker no longer reports is printed as a note, not a failure: a
# newer CLI that closes a gap must not fail one host and pass another.
#
# All specs go through one invocation: checked alone, a spec reports
# use.unresolvedPath on every `use`.
#
# Needs `allium` (cargo install allium-cli) at $MIN_VERSION or later, and jq.
# Behaviour is pinned by scripts/test-check-allium.sh.
#
# Run from the repo root:  ./scripts/check-allium.sh
set -euo pipefail

MIN_VERSION="3.6.1"
SPEC_DIR="docs/specs"
BASELINE="scripts/allium-check.baseline"

if ! command -v allium >/dev/null 2>&1; then
    cat >&2 <<MSG
check-allium: \`allium\` is not on PATH.
Install it with:  cargo install allium-cli --locked --version $MIN_VERSION
See docs/testing.md, "Toolchain and pre-push hook".
MSG
    exit 1
fi

# `allium 3.6.1 (language versions: 1, 2, 3)`: the version is the second field.
version="$(allium --version 2>/dev/null | awk '{print $2}')"
if ! printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
    echo "check-allium: cannot read a version from \`allium --version\` (got \"$version\")" >&2
    exit 1
fi
# Numeric comparison: a string compare would rank 3.10.0 below 3.6.1.
lowest="$(printf '%s\n%s\n' "$MIN_VERSION" "$version" | sort -V | head -n 1)"
if [ "$lowest" != "$MIN_VERSION" ]; then
    cat >&2 <<MSG
check-allium: allium $version is older than $MIN_VERSION, which this gate needs.
Upgrade with:  cargo install allium-cli --locked --version $MIN_VERSION
MSG
    exit 1
fi

shopt -s nullglob
specs=("$SPEC_DIR"/*.allium)
shopt -u nullglob
if [ "${#specs[@]}" -eq 0 ]; then
    echo "check-allium: no $SPEC_DIR/*.allium files found; run from the repo root" >&2
    exit 1
fi

# `allium check` exits 1 when it reports any warning or error, so its status
# alone cannot tell a finding from a crash. 0 and 1 both mean "it ran"; the JSON
# below decides. Anything else (2: no inputs) is a failure of the run itself.
rc=0
out="$(allium check "${specs[@]}")" || rc=$?
if [ "$rc" -ne 0 ] && [ "$rc" -ne 1 ]; then
    echo "check-allium: \`allium check\` exited $rc" >&2
    printf '%s\n' "$out" >&2
    exit 1
fi

# One JSON document per spec, concatenated. Count them, so an empty or truncated
# output cannot pass as "no diagnostics".
if ! docs="$(printf '%s' "$out" | jq -s 'length' 2>/dev/null)"; then
    echo "check-allium: \`allium check\` printed output that is not JSON:" >&2
    printf '%s\n' "$out" | head -n 20 >&2
    exit 1
fi
if [ "$docs" -ne "${#specs[@]}" ]; then
    echo "check-allium: \`allium check\` reported on $docs of ${#specs[@]} spec files" >&2
    exit 1
fi

# One line per diagnostic: severity, file, line, code, message (tab-separated).
diags="$(printf '%s' "$out" | jq -rs '
    .[] | .diagnostics[]? |
    [.severity, .location.file, (.location.line | tostring), .code, .message] | @tsv')"

# The baseline, without comments and blank lines: file<TAB>code<TAB>message.
baseline=""
if [ -f "$BASELINE" ]; then
    baseline="$(grep -v -e '^#' -e '^[[:space:]]*$' "$BASELINE" || true)"
fi

errors=""
unlisted=""
unlisted_keys=""
seen=""
infos=0
warnings=0
while IFS=$'\t' read -r severity file line code message; do
    [ -z "$severity" ] && continue
    key="$file"$'\t'"$code"$'\t'"$message"
    case "$severity" in
        error)
            errors="${errors}  ${file}:${line}  ${code}  ${message}"$'\n'
            ;;
        warning)
            warnings=$((warnings + 1))
            if printf '%s\n' "$baseline" | grep -qxF -- "$key"; then
                seen="${seen}${key}"$'\n'
            else
                unlisted="${unlisted}  ${file}:${line}  ${code}  ${message}"$'\n'
                unlisted_keys="${unlisted_keys}${key}"$'\n'
            fi
            ;;
        *)
            infos=$((infos + 1))
            ;;
    esac
done <<EOF
$diags
EOF

status=0

if [ -n "$errors" ]; then
    echo "check-allium: allium check reports errors:" >&2
    printf '%s' "$errors" >&2
    echo "Fix them in the spec; errors are never baselined." >&2
    status=1
fi

if [ -n "$unlisted" ]; then
    [ "$status" -ne 0 ] && echo >&2
    echo "check-allium: allium check reports warnings not in $BASELINE:" >&2
    printf '%s' "$unlisted" >&2
    cat >&2 <<MSG
Fix the spec. Only when the warning is a checker gap (the spec is right and
allium misreads it), add a comment beside the declaration saying why it is
expected, and add these lines (tab-separated) to $BASELINE:
MSG
    printf '%s' "$unlisted_keys" >&2
    status=1
fi

# A listed warning the checker no longer reports: say so, but do not fail.
stale=""
while IFS= read -r entry; do
    [ -z "$entry" ] && continue
    if ! printf '%s' "$seen" | grep -qxF -- "$entry"; then
        stale="${stale}  ${entry}"$'\n'
    fi
done <<EOF
$baseline
EOF
if [ -n "$stale" ]; then
    echo "check-allium: note: $BASELINE lists warnings allium no longer reports; remove them:"
    printf '%s' "$stale"
fi

if [ "$status" -ne 0 ]; then
    exit "$status"
fi

echo "check-allium: ${#specs[@]} specs, no errors, $warnings baselined warnings, $infos info (allium $version)"
