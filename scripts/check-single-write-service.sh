#!/usr/bin/env bash
# Flag a src/service/ function that makes more than one store write
# (`self.db.<write>(`) for one logical change. Two writes are two reducer
# calls: a failure or a concurrent reader between them sees half a change.
# Findings of this class came back three times (S1 10-22 and 10-28, then R3
# 10-31: TaskService::update_task, EpicService::update_epic,
# WorkPackageService::update_work_package). The fix is one module reducer.
#
# A write is a call on a `db.` receiver: `self.db.<m>(`, a bare `db.<m>(` in a
# free function, or either wrapped by rustfmt across lines (`self\n.db\n.m(`).
# The count is per `fn` at any indent, so free functions are covered too. A
# call is a READ when its method name starts with get_/list_/find_/is_/has_/
# count_/my_/query_/read_ or ends in _exists, or is in READS below. Everything
# else counts as a write. Limit: a single write inside a loop counts once.
#
# Sanctioned exceptions go in scripts/single-write-service.allow, one
# `path::function  N  reason` per line, where N is the exact number of writes
# the function makes today. A function that gains a write fails (N no longer
# matches); one that is not multi-write any more but is still listed fails too,
# so the list cannot rot.
#
# Behaviour is pinned by scripts/test-check-single-write-service.sh.
# Run from the repo root. Exits non-zero on a violation.
set -euo pipefail

ALLOW=${SINGLE_WRITE_ALLOW:-scripts/single-write-service.allow}
READS=" admin_snapshot dispatch_request session_request clone "

hits=$(
    find src/service -name '*.rs' -type f 2>/dev/null | sort | while IFS= read -r f; do
        # Test files and inline test modules are not service code.
        case "$f" in */tests.rs | */tests/* | */fixtures.rs) continue ;; esac
        awk -v reads="$READS" '
            function count_writes(body,    m, n, pre) {
                # Join wrapped receivers: `self\n.db\n.m(` -> `self.db.m(`.
                gsub(/[ \t]*\.[ \t]*/, ".", body)
                n = 0
                while (match(body, /(self\.)?db\.[A-Za-z0-9_]+[ \t]*\(/)) {
                    pre = (RSTART > 1) ? substr(body, RSTART - 1, 1) : " "
                    m = substr(body, RSTART, RLENGTH)
                    body = substr(body, RSTART + RLENGTH)
                    # `my_db.x(` and `a.db.x(` are not this receiver.
                    if (pre ~ /[A-Za-z0-9_.]/) continue
                    sub(/^(self\.)?db\./, "", m)
                    sub(/[ \t]*\($/, "", m)
                    if (m ~ /^(get|list|find|is|has|count|my|query|read)_/ || m ~ /_exists$/) continue
                    if (index(reads, " " m " ") > 0) continue
                    n++
                }
                return n
            }
            function flush(    n) {
                if (fn != "") {
                    n = count_writes(buf)
                    if (n > 1) printf "%s::%s %d\n", FILENAME, fn, n
                }
                buf = ""
            }
            /^#\[cfg\(test\)\]/ { flush(); fn = ""; exit }
            /^[ \t]*(pub(\([a-z]+\))? )?(async )?fn [A-Za-z0-9_]+/ {
                flush()
                s = $0
                sub(/^[ \t]*(pub(\([a-z]+\))? )?(async )?fn /, "", s)
                sub(/[^A-Za-z0-9_].*/, "", s)
                fn = s
            }
            {
                line = $0
                sub(/\/\/.*/, "", line)
                buf = buf " " line
            }
            END { flush() }
        ' "$f"
    done
)

allowed=$(sed -e 's/#.*//' "$ALLOW" 2>/dev/null | awk 'NF { print $1 " " $2 }' | sort -u || true)
bad=""
while IFS= read -r row; do
    [ -z "$row" ] && continue
    key=${row%% *}
    n=${row#* }
    want=$(awk -v k="$key" '$1 == k { print $2; exit }' <<<"$allowed")
    if [ -z "$want" ]; then
        bad="${bad}${row} writes\n"
    elif [ "$want" != "$n" ]; then
        bad="${bad}${key} makes ${n} writes, the allow-list says ${want}\n"
    fi
done <<<"$hits"

stale=""
while IFS= read -r line; do
    [ -z "$line" ] && continue
    key=${line%% *}
    if ! grep -qF -- "${key} " <<<"$hits"; then
        stale="${stale}${key}\n"
    fi
done <<<"$allowed"

if [ -n "$bad" ]; then
    echo "check-single-write-service: service functions with more than one store write:" >&2
    printf '%b' "$bad" >&2
    echo "Make it one module reducer call, or list it in $ALLOW as path::fn  N  reason." >&2
    exit 1
fi
if [ -n "$stale" ]; then
    echo "check-single-write-service: allow-list entries that are no longer multi-write:" >&2
    printf '%b' "$stale" >&2
    echo "Remove them from $ALLOW." >&2
    exit 1
fi
echo "check-single-write-service: no unlisted multi-write service functions"
