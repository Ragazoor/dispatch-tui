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
# else counts as a write.
#
# Beyond direct calls (G4): a write inside a `for`/`while`/`loop` body counts
# twice (it runs once per item); a `self.<helper>(` call adds the helper's own
# write count, once per call (twice inside a loop), one level deep; and
# `let store = &self.db;` / `let s = self.db.clone();` / `let s =
# Arc::clone(&self.db);` make `store`/`s` count as the receiver. Every `fn`
# header form is recognised (`pub(in path)`, `async`, `const`, `unsafe`).
#
# `#[cfg(test)]` skips only the item it marks (a braced block, or a statement
# up to its `;`), never the rest of the file (G6). A `//` inside a string is not
# a comment, and a brace inside a string is not counted (G7). A helper is looked
# up only among the `fn`s of the same `impl` block, so a same-named helper in
# another block is not charged (G7).
#
# Limits, accepted: only `src/service/` is scanned; loops written as iterator
# closures (`.for_each(|x| {..})`, `.map`) are not loops here; a loop header
# wrapped over several lines before its `{` is missed; helpers are followed
# one level and by name within the same file (`self.x(` only, not `Self::x(`
# or a call through another field); aliases are `let` bindings of `db` only.
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
            # Writes in `body` (a joined, comment-free function body with
            # @LOOP@ / @ENDLOOP@ markers around loop bodies). A write inside a
            # loop counts twice: it runs once per item. Calls to `self.<name>(`
            # are recorded in calls[] with the same weight, for the helper pass.
            function count_writes(body, recvre,    n, seg, mark, level, w, s, m, pre, t, name) {
                # Join wrapped receivers: `self\n.db\n.m(` -> `self.db.m(`.
                gsub(/[ \t]*\.[ \t]*/, ".", body)
                n = 0
                level = 0
                while (length(body) > 0) {
                    if (match(body, /@(END)?LOOP@/)) {
                        seg = substr(body, 1, RSTART - 1)
                        mark = substr(body, RSTART, RLENGTH)
                        body = substr(body, RSTART + RLENGTH)
                    } else {
                        seg = body
                        mark = ""
                        body = ""
                    }
                    w = (level > 0) ? 2 : 1
                    s = seg
                    while (match(s, recvre)) {
                        pre = (RSTART > 1) ? substr(s, RSTART - 1, 1) : " "
                        m = substr(s, RSTART, RLENGTH)
                        s = substr(s, RSTART + RLENGTH)
                        # `my_db.x(` and `a.db.x(` are not this receiver.
                        if (pre ~ /[A-Za-z0-9_.]/) continue
                        sub(/^[A-Za-z0-9_.]*\./, "", m)
                        sub(/[ \t]*\($/, "", m)
                        if (m ~ /^(get|list|find|is|has|count|my|query|read)_/ || m ~ /_exists$/) continue
                        if (index(reads, " " m " ") > 0) continue
                        n += w
                    }
                    t = seg
                    while (match(t, /self\.[A-Za-z0-9_]+[ \t]*\(/)) {
                        pre = (RSTART > 1) ? substr(t, RSTART - 1, 1) : " "
                        name = substr(t, RSTART + 5, RLENGTH - 5)
                        t = substr(t, RSTART + RLENGTH)
                        if (pre ~ /[A-Za-z0-9_.]/) continue
                        sub(/[ \t]*\($/, "", name)
                        calls[name] += w
                    }
                    if (mark == "@LOOP@") level++
                    else if (mark == "@ENDLOOP@" && level > 0) level--
                }
                return n
            }
            # `let store = &self.db;` and `let s = self.db.clone();` rebind the
            # receiver: the alias writes are the same writes.
            function receiver_re(body,    alts, t, a) {
                alts = "db"
                t = body
                gsub(/[ \t]*\.[ \t]*/, ".", t)
                while (match(t, /let[ \t]+(mut[ \t]+)?[A-Za-z0-9_]+[^=;]*=[ \t]*(([A-Za-z0-9_]+::)*clone\()?&?(mut[ \t]+)?(self\.)?db(\.clone\(\))?\)?[ \t]*;/)) {
                    a = substr(t, RSTART, RLENGTH)
                    t = substr(t, RSTART + RLENGTH)
                    sub(/^let[ \t]+(mut[ \t]+)?/, "", a)
                    sub(/[^A-Za-z0-9_].*/, "", a)
                    if (a != "") alts = alts "|" a
                }
                return "(self[.])?(" alts ")[.][A-Za-z0-9_]+[ \t]*[(]"
            }
            # The line without its `//` comment. A `//` inside a string literal
            # ("http://x") is not a comment.
            function strip_comment(l,    i, c, out, instr) {
                out = ""
                instr = 0
                for (i = 1; i <= length(l); i++) {
                    c = substr(l, i, 1)
                    if (instr) {
                        out = out c
                        if (c == "\\") { i++; out = out substr(l, i, 1) }
                        else if (c == "\"") instr = 0
                    } else if (c == "\"") {
                        instr = 1
                        out = out c
                    } else if (c == "/" && substr(l, i + 1, 1) == "/") {
                        break
                    } else if (c == "\047" && substr(l, i + 2, 1) == "\047") {
                        out = out substr(l, i, 3)
                        i += 2
                    } else {
                        out = out c
                    }
                }
                return out
            }
            # String contents blanked, so a brace inside a literal is not counted.
            function blank_strings(l) {
                gsub(/"([^"\\]|\\.)*"/, "\"\"", l)
                return l
            }
            function flush(    n, k, hs) {
                if (fn != "") {
                    delete calls
                    n = count_writes(buf, receiver_re(buf))
                    hs = ""
                    for (k in calls) hs = hs k ":" calls[k] " "
                    nfn++
                    fname[nfn] = fn
                    fown[nfn] = n
                    fcalls[nfn] = hs
                    fimpl[nfn] = fn_impl
                }
                buf = ""
            }
            BEGIN {
                FNHEAD = "^[ \t]*(pub([(][^)]*[)])?[ \t]+)?((const|async|unsafe)[ \t]+)*fn[ \t]+[A-Za-z0-9_]+"
                LOOPHEAD = "^[ \t]*(.[a-z_]+:[ \t]*)?((for|while)[ \t]|loop[ \t]*[{])"
            }
            # `#[cfg(test)]` skips the one item it marks (a braced block, or a
            # statement ending in `;`), not the rest of the file (G6).
            skipping {
                code = blank_strings(strip_comment($0))
                tmp = code
                o = gsub(/[{]/, "{", tmp)
                c = gsub(/[}]/, "}", tmp)
                if (o > 0) sk_brace = 1
                sk_depth += o - c
                if (sk_depth <= 0 && (sk_brace || code ~ /;[ \t]*$/)) skipping = 0
                next
            }
            /^[ \t]*#\[cfg\(test\)\]/ { sk_armed = 1; next }
            sk_armed && /^[ \t]*(#\[.*)?$/ { next }
            sk_armed {
                sk_armed = 0
                skipping = 1
                sk_depth = 0
                sk_brace = 0
                code = blank_strings(strip_comment($0))
                tmp = code
                o = gsub(/[{]/, "{", tmp)
                c = gsub(/[}]/, "}", tmp)
                if (o > 0) sk_brace = 1
                sk_depth = o - c
                if (sk_depth <= 0 && (sk_brace || code ~ /;[ \t]*$/)) skipping = 0
                next
            }
            $0 ~ FNHEAD {
                flush()
                depth_loops = 0
                s = $0
                match(s, /^[ \t]*(pub([(][^)]*[)])?[ \t]+)?((const|async|unsafe)[ \t]+)*fn[ \t]+/)
                s = substr(s, RLENGTH + 1)
                sub(/[^A-Za-z0-9_].*/, "", s)
                fn = s
                fn_impl = cur_impl
                nloops = 0
            }
            {
                line = strip_comment($0)
                tmp = blank_strings(line)
                opens = gsub(/[{]/, "{", tmp)
                closes = gsub(/[}]/, "}", tmp)
                lead = ""
                pre_marks = ""
                post_marks = ""
                if (line ~ LOOPHEAD && opens > 0) {
                    if (opens - closes > 0) {
                        loopat[++nloops] = depth
                        pre_marks = " @LOOP@ "
                    } else {
                        lead = " @LOOP@ "
                        post_marks = " @ENDLOOP@ "
                    }
                }
                if (line ~ /^[ \t]*(unsafe[ \t]+)?impl[ \t<]/ && !cur_impl) {
                    cur_impl = ++nimpl
                    impl_depth = depth
                    impl_open = 0
                }
                depth += opens - closes
                if (cur_impl) {
                    if (depth > impl_depth) impl_open = 1
                    else if (impl_open) cur_impl = 0
                }
                while (nloops > 0 && depth <= loopat[nloops]) {
                    post_marks = post_marks " @ENDLOOP@ "
                    nloops--
                }
                # The loop marker goes after the header text, so a header that
                # itself holds a write is not charged to the body.
                buf = buf " " lead line pre_marks post_marks
            }
            END {
                flush()
                for (i = 1; i <= nfn; i++) {
                    total = fown[i]
                    nh = split(fcalls[i], hs, " ")
                    for (h = 1; h <= nh; h++) {
                        split(hs[h], kv, ":")
                        best = 0
                        for (j = 1; j <= nfn; j++)
                            if (j != i && fimpl[j] == fimpl[i] && fname[j] == kv[1] && fown[j] > best) best = fown[j]
                        total += kv[2] * best
                    }
                    if (total > 1) printf "%s::%s %d\n", FILENAME, fname[i], total
                }
            }
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
