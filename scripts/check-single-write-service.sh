#!/usr/bin/env bash
# Flag a src/service/ function that makes more than one store write
# (`self.db.<write>(`) for one logical change. Two writes are two reducer
# calls: a failure or a concurrent reader between them sees half a change.
# Findings of this class came back three times (S1 10-22 and 10-28, then R3
# 10-31: TaskService::update_task, EpicService::update_epic,
# WorkPackageService::update_work_package). The fix is one module reducer.
#
# A call is a READ when its method name starts with get_/list_/find_/is_/has_/
# count_/my_/query_/read_ or ends in _exists, or is in READS below. Everything
# else counts as a write.
#
# Sanctioned exceptions go in scripts/single-write-service.allow, one
# `path::function  reason` per line. A function that is not multi-write any
# more but is still listed fails the check, so the list cannot rot.
#
# Behaviour is pinned by scripts/test-check-single-write-service.sh.
# Run from the repo root. Exits non-zero on a violation.
set -euo pipefail

ALLOW=${SINGLE_WRITE_ALLOW:-scripts/single-write-service.allow}
READS=" admin_snapshot dispatch_request "

hits=$(
    find src/service -name '*.rs' -type f 2>/dev/null | sort | while IFS= read -r f; do
        # Test files and inline test modules are not service code.
        case "$f" in */tests.rs | */tests/* | */fixtures.rs) continue ;; esac
        awk -v reads="$READS" '
            function flush() {
                if (fn != "" && n > 1) printf "%s::%s %d\n", FILENAME, fn, n
                n = 0
            }
            /^#\[cfg\(test\)\]/ { flush(); fn = ""; exit }
            /^    (pub(\([a-z]+\))? )?(async )?fn [A-Za-z0-9_]+/ {
                flush()
                s = $0
                sub(/^    (pub(\([a-z]+\))? )?(async )?fn /, "", s)
                sub(/[^A-Za-z0-9_].*/, "", s)
                fn = s
                next
            }
            /^(pub(\([a-z]+\))? )?(async )?fn [A-Za-z0-9_]+/ { flush(); fn = ""; next }
            {
                line = $0
                sub(/\/\/.*/, "", line)
                while (match(line, /self\.db\.[A-Za-z0-9_]+\(/)) {
                    m = substr(line, RSTART + 8, RLENGTH - 9)
                    line = substr(line, RSTART + RLENGTH)
                    if (m ~ /^(get|list|find|is|has|count|my|query|read)_/ || m ~ /_exists$/) continue
                    if (index(reads, " " m " ") > 0) continue
                    n++
                }
            }
            END { flush() }
        ' "$f"
    done
)

allowed=$(sed -e 's/#.*//' "$ALLOW" 2>/dev/null | awk 'NF { print $1 }' | sort -u || true)
bad=""
while IFS= read -r row; do
    [ -z "$row" ] && continue
    key=${row%% *}
    if ! grep -qxF "$key" <<<"$allowed"; then
        bad="${bad}${row} writes\n"
    fi
done <<<"$hits"

stale=""
while IFS= read -r key; do
    [ -z "$key" ] && continue
    if ! grep -qF -- "${key} " <<<"$hits"; then
        stale="${stale}${key}\n"
    fi
done <<<"$allowed"

if [ -n "$bad" ]; then
    echo "check-single-write-service: service functions with more than one store write:" >&2
    printf '%b' "$bad" >&2
    echo "Make it one module reducer call, or list it in $ALLOW with a reason." >&2
    exit 1
fi
if [ -n "$stale" ]; then
    echo "check-single-write-service: allow-list entries that are no longer multi-write:" >&2
    printf '%b' "$stale" >&2
    echo "Remove them from $ALLOW." >&2
    exit 1
fi
echo "check-single-write-service: no unlisted multi-write service functions"
