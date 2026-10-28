# resume_task Host Gate

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `resume_task` refuse what the board will refuse, and never let a resume request vanish without a message.

## Context

This work package addresses finding N3 from `docs/plans/2026-10-28-codebase-review.md`. Read `docs/mcp-tool-design.md` before changing the tool.

## Findings

### 🟡 Medium — N3: `resume_task` answers success for a task the board will refuse (`src/mcp/handlers/tasks/resume.rs:45-96`)

**Issue:** `ResumeTaskViaMcp` in `docs/specs/mcp-task-tools.allium` requires `task.is_locally_owned` and lists "its worktree is on another machine" as a tool error. `resume.rs::refusal` checks status, worktree and window only (its comment leaves the host gate to the board), then `handle_resume_task` answers "resume requested … the board is restarting its agent". The board (`src/tui/update/agent.rs::handle_resume_task`, around lines 646-689) refuses on the status line only, and returns `vec![]` silently when the task left the board's view or changed between the read and the event. An agent resuming a teammate's task waits for a session that never starts.

**Fix (option a):** Plumb the host id (`HostStore::host_id`, `src/store/queries/settings.rs`) into `McpState`; refuse in `refusal` when the task's host is not this one. Make the board's silent branches set a status message.

## Decisions

### N3: code or spec (`src/mcp/handlers/tasks/resume.rs`)

**Question:** Should the tool enforce the host gate the spec names, or should the spec change to match today's code?

**Options:**
- (a) Enforce in the tool, as above. Costs plumbing a host id into `McpState`; matches the spec; the agent gets a real error.
- (b) Change the spec: the tool only requests, the board decides. Costs an honest reply wording ("requested; the board may refuse") and no way for the agent to learn the outcome.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up. The board's silent `vec![]` branches should get a status message under either option.

## Changes

| File | Change |
|------|--------|
| `src/mcp/handlers/tasks/resume.rs` | Host gate in `refusal` (option a) or reply wording (option b) |
| `src/mcp/` state | Host id on `McpState` (option a) |
| `src/tui/update/agent.rs` | Status message on every refused resume |
| `docs/specs/mcp-task-tools.allium` | Only for option b |
| `src/mcp/handlers/tests/` | Another host's task is refused (a); silent-branch message tests |

## Verification

- [ ] New tests fail first, then pass
- [ ] `cargo test --no-fail-fast`
- [ ] `cargo fmt --check && ./scripts/check-doc-paths.sh`
