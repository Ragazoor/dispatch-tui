#!/usr/bin/env bash
# Guard against wall-clock dependence in test code. Tests must await a
# deterministic completion signal (oneshot / Notify / an mpsc event such as
# McpEvent) or inject a clock/threshold — never sleep on the wall clock or
# measure it, both of which are flaky on slow CI and needlessly slow. See
# docs/conventions.md ("No `tokio::time::sleep` in tests").
#
# Two checks, both in *test code* only:
#
#  1. `tokio::time::sleep(`. Unconditional: the allow marker below does not
#     exempt it. Production use is legitimate — `next_loop_event` in
#     src/runtime/event_loop.rs awaits it for the frame-wait deadline.
#  2. `std::thread::sleep(` and `.elapsed()`. Production use of both is
#     legitimate — `src/process.rs` and `src/runtime/mod.rs` sleep, and
#     TTL/interval checks all over the TUI read `.elapsed()`.
#
# "Test code" means a test file — anything under tests/, under a src/**/tests/
# directory, or named tests.rs or *_tests.rs — **or** an inline `#[cfg(test)] mod <name> { …
# }` block inside a production file. Inline modules used to be a documented
# blind spot; they are now tracked by the awk pass below, which opens a region
# at a top-level `#[cfg(test)]` immediately followed by `mod <name> {` (with or
# without a visibility prefix) and closes it at the matching column-0 `}`. A
# `#[cfg(test)]` on anything other than a module (a test-only struct, say) does
# not open a region, and neither does a `#[cfg(test)] mod tests;` file module —
# the file it names is caught by the test-file rule instead. The column-0 rule
# leans on rustfmt: everything inside a module is indented, so the only way to
# close a region early is a multi-line string literal with a line starting at
# column 0 with `}`. That under-reports rather than false-positives.
#
# Escape hatch for check 2 only: an `allow-test-sleep:` comment on the offending
# line or the line directly above it, carrying a reason. It exists for the one
# shape a grep cannot distinguish from a fixed wall-clock dependence — a poll
# loop that checks a condition against a deadline, where only the failure path
# pays the full wait (see tests/tmux_harness/mod.rs). It is not a way to keep a
# fixed sleep or a timing assertion: if removing the surrounding condition
# check would leave the test still passing, it is a fixed sleep and must go.
#
# The trailing "(" in the sleep patterns and the leading "." in `.elapsed()`
# match call sites only, so doc-comment mentions of the rule and test names
# ending in `_elapsed()` are not flagged.
#
# Behaviour is pinned by scripts/test-check-no-test-sleep.sh.
#
# Run from the repo root. Exits non-zero if any match is found.
set -euo pipefail

# Wall-clock reads and sleeps in test code, one "kind:path:line:text" per line,
# where kind is `tokio` (check 1) or `wall` (check 2). The awk pass decides what
# counts as test code; the bash loop below applies the allow-marker exemption to
# `wall` hits only.
# `-exec … +` runs nothing at all when no file matches, so no emptiness guard is
# needed; if the arg list is long enough to split, FNR == 1 resets per-file state.
test_hits=$(
    find src tests -name '*.rs' -type f 2>/dev/null -exec awk '
        FNR == 1 {
            in_mod = 0
            pending = 0
            # Whole-file test code: tests/…, src/**/tests/…, or …/tests.rs, …/*_tests.rs.
            test_file = (FILENAME ~ /(^|\/)tests\//) || (FILENAME ~ /(^|\/)([a-z0-9_]+_)?tests\.rs$/)
        }
        !test_file {
            if ($0 ~ /^#\[cfg\(test\)\]$/) { pending = 1; next }
            if (pending) {
                pending = 0
                if ($0 ~ /^(pub(\([a-z:]+\))?[[:space:]]+)?mod [A-Za-z0-9_]+ \{[[:space:]]*$/) { in_mod = 1; next }
            }
            if (in_mod && $0 ~ /^\}/) { in_mod = 0; next }
        }
        !(test_file || in_mod) { next }
        index($0, "tokio::time::sleep(") > 0 {
            printf "tokio:%s:%d:%s\n", FILENAME, FNR, $0
            next
        }
        index($0, "std::thread::sleep(") > 0 || index($0, ".elapsed()") > 0 {
            printf "wall:%s:%d:%s\n", FILENAME, FNR, $0
        }
    ' {} +
)

sleeps=""
violations=""
while IFS= read -r tagged; do
    [ -z "$tagged" ] && continue
    kind=${tagged%%:*}
    hit=${tagged#*:}
    if [ "$kind" = tokio ]; then
        sleeps="${sleeps}${hit}"$'\n'
        continue
    fi
    file=${hit%%:*}
    rest=${hit#*:}
    line=${rest%%:*}
    from=$line
    [ "$line" -gt 1 ] && from=$((line - 1))
    # An allow marker on the call-site line or the line directly above it.
    if sed -n "${from},${line}p" "$file" | grep -qF 'allow-test-sleep:'; then
        continue
    fi
    violations="${violations}${hit}"$'\n'
done <<EOF
$test_hits
EOF

if [ -n "${sleeps//[$'\n']/}" ]; then
    echo "check-no-test-sleep: forbidden tokio::time::sleep() in test code:" >&2
    printf '%s' "$sleeps" >&2
    echo >&2
    echo "Await a deterministic completion signal (oneshot/Notify/mpsc event)" >&2
    echo "or inject a clock instead of sleeping. The allow-test-sleep marker" >&2
    echo "does not apply here. See docs/conventions.md." >&2
    exit 1
fi

if [ -n "${violations//[$'\n']/}" ]; then
    echo "check-no-test-sleep: test code must not sleep on or measure the wall clock:" >&2
    printf '%s' "$violations" >&2
    echo >&2
    echo "Production std::thread::sleep and .elapsed() are fine; test code must" >&2
    echo "neither sleep nor assert on measured elapsed time. Inject the" >&2
    echo "threshold/clock, await a completion signal, or bound the wait" >&2
    echo "structurally (tokio::time::timeout, Receiver::recv_timeout) instead." >&2
    echo "A deadline-bounded poll step may carry an 'allow-test-sleep: <why>'" >&2
    echo "comment on or directly above the call. See docs/conventions.md." >&2
    exit 1
fi

echo "check-no-test-sleep: no unjustified wall-clock use in test code"
