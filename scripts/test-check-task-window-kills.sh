#!/usr/bin/env bash
# Self-test for check-task-window-kills.sh.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$tmp/src/service"
printf 'fn a() { crate::dispatch::kill_task_window(&w, &r); }\n' > "$tmp/src/service/ok.rs"
"$here/check-task-window-kills.sh" "$tmp" || { echo "FAIL: clean tree rejected" >&2; exit 1; }

printf 'fn a() { crate::tmux::kill_window(&w, &r); }\n' > "$tmp/src/service/bad.rs"
if "$here/check-task-window-kills.sh" "$tmp" 2>/dev/null; then
  echo "FAIL: plain kill_window not caught" >&2; exit 1
fi
rm "$tmp/src/service/bad.rs"

printf 'fn a() { tmux::kill_window_if_present(&w, &r); }\n' > "$tmp/src/service/bad2.rs"
if "$here/check-task-window-kills.sh" "$tmp" 2>/dev/null; then
  echo "FAIL: plain kill_window_if_present not caught" >&2; exit 1
fi
echo "ok"
