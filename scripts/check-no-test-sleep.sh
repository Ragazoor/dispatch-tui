#!/usr/bin/env bash
# Guard against `tokio::time::sleep` in test code. Async tests must await a
# deterministic completion signal (oneshot / Notify / an mpsc event such as
# McpEvent) or inject a clock — never sleep on the wall clock, which is flaky on
# slow CI and needlessly slow. See docs/conventions.md ("No `tokio::time::sleep`
# in tests").
#
# Production `std::thread::sleep` (e.g. src/process.rs, src/runtime/mod.rs) is
# legitimate and is never matched by this check. The trailing "(" in the pattern
# matches call sites only, so doc-comment mentions of the rule are not flagged.
#
# Run from the repo root. Exits non-zero if any match is found.
set -euo pipefail

if hits=$(grep -rnF --include='*.rs' 'tokio::time::sleep(' src tests 2>/dev/null); then
    echo "check-no-test-sleep: forbidden tokio::time::sleep() found:" >&2
    echo "$hits" >&2
    echo >&2
    echo "Await a deterministic completion signal (oneshot/Notify/mpsc event)" >&2
    echo "or inject a clock instead of sleeping. See docs/conventions.md." >&2
    exit 1
fi

echo "check-no-test-sleep: no tokio::time::sleep in test code"
