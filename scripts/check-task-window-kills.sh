#!/usr/bin/env bash
# Fail when a task window is killed with a plain tmux kill instead of
# dispatch::kill_task_window (or kill_task_window_if_present), which spare the
# user's shell in a split leftover window. See "Leftover window" in
# docs/specs/split-pane.allium.
#
# Only the files below may call tmux::kill_window / kill_window_if_present:
# the tmux module itself, the two helpers' home, the editor window (not a
# task window), the split-swap cleanup of a window dispatch just made, and an
# epic orchestrator's window (not a task window, and never split).
# Test code is out of scope.
#
# Usage: check-task-window-kills.sh [root]   (root defaults to the repo)
set -euo pipefail
root="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
cd "$root"

allowed='^src/(tmux\.rs|dispatch/split_panes\.rs|dispatch/orchestrator\.rs|runtime/editor\.rs):'
hits=$(grep -rnE '(tmux::|\b)kill_window(_if_present)?\(' src --include=*.rs \
  | grep -vE '^[^:]*/tests?(/|\.rs)|tests\.rs:' \
  | grep -vE '^[^:]*:[0-9]+:\s*(//|///)' \
  | grep -vE '^[^:]*:[0-9]+:\s*(pub )?fn kill_window' \
  | grep -vE "$allowed" || true)

if [ -n "$hits" ]; then
  echo "plain tmux window kill outside kill_task_window:" >&2
  echo "$hits" >&2
  echo "Use dispatch::kill_task_window / kill_task_window_if_present." >&2
  exit 1
fi
