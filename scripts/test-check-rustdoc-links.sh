#!/usr/bin/env bash
# Behavioural test for scripts/check-rustdoc-links.sh, run against canned
# rustdoc output so it needs no compile.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECKER="$SCRIPT_DIR/check-rustdoc-links.sh"
WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT
fail=0

expect() {
    local want="$1" label="$2" log="$WORKDIR/log.txt"
    cat >"$log"
    local got=0
    "$CHECKER" "$log" >/dev/null 2>&1 || got=$?
    if [[ "$got" != "$want" ]]; then
        echo "FAIL: $label (want exit $want, got $got)" >&2
        fail=1
    fi
}

expect 0 'clean output passes' <<'LOG'
    Finished `dev` profile in 1.0s
LOG

expect 1 'an unresolved link in src fails' <<'LOG'
error: unresolved link to `render`
   --> src/agent_tree/state.rs:141:45
LOG

expect 1 'an unresolved link reported as a warning fails' <<'LOG'
warning: unresolved link to `render`
   --> src/agent_tree/state.rs:141:45
LOG

expect 0 'an unresolved link in generated bindings is ignored' <<'LOG'
error: unresolved link to `create_task:create_task_then`
  --> src/spacetime/bindings/create_task_reducer.rs:35:19
LOG

expect 0 'a private-item link warning is not a broken link' <<'LOG'
warning: public documentation for `handle_key` links to private item `dispatch_key`
  --> src/agent_tree/keys.rs:67:13
LOG

expect 1 'a real finding survives next to ignored bindings ones' <<'LOG'
error: unresolved link to `create_task:create_task_then`
  --> src/spacetime/bindings/create_task_reducer.rs:35:19
error: unresolved link to `foo`
   --> src/git.rs:211:8
LOG

if ((fail)); then
    exit 1
fi
echo "test-check-rustdoc-links: all assertions passed"
