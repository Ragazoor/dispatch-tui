# Agent Launch Shell Quoting

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make agent launch work for repos whose path contains a space, `'`, `$` or other shell metacharacters.

## Context

This work package addresses finding 2 of the 2026-10-17 code review
(`docs/plans/2026-10-17-codebase-review.md`).

## Findings

### 🔴 `--mcp-config` path breaks the `bash -c` quoting (`src/dispatch/agents.rs:363`, `src/dispatch/caller_identity.rs:99`)

**Issue:** `prompt_launch_command` builds
`bash -c 'prompt=$(cat .claude-prompt) && rm -f .claude-prompt && "$0" {launch_flags} -- "$prompt"' {claude}`.
`launch_flags` ends with `mcp_config_flag`'s ` --mcp-config <shell_quote(path)>`.
When the path needs quoting, `shell_quote` wraps it in `'…'`, and those quotes
close the outer single-quoted body. The path is
`<repo>/.git/worktrees/<name>/dispatch-mcp.json`, so a repo at
`~/My Projects/app` gives a bash syntax error or a wrong argument and claude
never starts; a `'` lets path text run as shell. The doc on `claude_quoted`
claims exactly one quoting layer applies at every site — false for this flag.
`resume_agent` is unaffected (no `bash -c` wrapper).

**Fix:** Pass the config path as a positional argument after `$0` and refer
to it as `"$1"` inside the body (as `$0` already does for the claude binary),
so it needs no quoting inside the body. Check every other flag inside
`launch_flags` for the same problem. Correct the `claude_quoted` doc.

## Changes

| File | Change |
|------|--------|
| `src/dispatch/agents.rs` | `prompt_launch_command` takes the MCP config path as a positional; doc fix |
| `src/dispatch/caller_identity.rs` | `mcp_config_flag` returns the raw path (or a struct) instead of a pre-quoted flag |
| `src/dispatch/tests/agent_launch.rs` | New cases |

## Verification

- [ ] Test first: launch command for a worktree path with a space, and one with a `'`, parses under `bash -n` / runs `"$0"` with the exact path (use a fake `claude` that echoes its argv)
- [ ] Existing launch tests still pass
- [ ] `cargo test --no-fail-fast` and the task's verify command pass
