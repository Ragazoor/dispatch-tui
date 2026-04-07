# Review Agent UX Design

## Context

Dispatched review agents currently auto-post reviews to GitHub via `gh pr review`. This removes human judgment from the loop — the user has no chance to curate, dismiss, or discuss findings before they appear on the PR.

The goal is a **long-lived review companion** model: the agent analyzes the PR, presents findings in its Claude Code session, signals the TUI that findings are ready, and waits. The user jumps into the session, reads findings, discusses with the agent, and decides what to post. The session stays alive for re-reviews as the PR evolves.

## Scope

Applies to:
- **Review Board — Reviewer mode**: Regular PRs needing review
- **Review Board — Dependabot mode**: Bot-generated dependency update PRs
- **Security Board**: Fix agents (same lifecycle signaling, different prompt)

Does NOT apply to:
- **Review Board — Author mode**: User's own PRs; no dispatch needed

## Data Model

### ReviewAgentStatus enum

```rust
pub enum ReviewAgentStatus {
    Reviewing,      // Agent is actively analyzing
    FindingsReady,  // Analysis complete, user should look
    Idle,           // Waiting for user action or re-review
}
```

### Model changes

`ReviewPr` and `SecurityAlert` gain an `agent_status: Option<ReviewAgentStatus>` field. When `tmux_window` is `None`, `agent_status` is always `None`.

### DB changes

New `agent_status` TEXT column (nullable) on `review_prs`, `bot_prs`, and `security_alerts` tables. Values: `"reviewing"`, `"findings_ready"`, `"idle"`, or NULL.

Migration: `ALTER TABLE` to add the column with NULL default.

## MCP Tool: `update_review_status`

```
Name: "update_review_status"
Description: "Update the agent status for a dispatched review or fix agent."
Input:
  - repo: string (required) — GitHub owner/repo
  - number: integer (required) — PR or alert number
  - status: string (required) — "reviewing", "findings_ready", or "idle"
```

**Handler behavior:**
1. Search `review_prs`, `bot_prs`, and `security_alerts` for `(repo, number)` WHERE `tmux_window IS NOT NULL`
2. Validate exactly one match found
3. Update `agent_status` in DB
4. Call `state.notify()` to trigger TUI refresh
5. If status is `findings_ready`, send a new `McpEvent::ReviewReady { repo, number }` variant to flash the card (existing `MessageSent` takes a `TaskId` which review PRs don't have)

## Prompts

### Regular PR (Reviewer mode)

```
You are reviewing PR #{number} in {github_repo}: {title}

{body}

Run `/review-pr {number}` to perform a comprehensive code review.

After the review completes, call the `update_review_status` MCP tool:
  update_review_status(repo="{github_repo}", number={number}, status="findings_ready")

Wait for the user.
```

### Dependabot PR

```
You are reviewing a dependency update PR #{number} in {github_repo}: {title}

{body}

This is an automated dependency update. Run `/review-pr {number}` to review.

After the review completes, call the `update_review_status` MCP tool:
  update_review_status(repo="{github_repo}", number={number}, status="findings_ready")

Wait for the user.
```

### Fix agent (Security Board)

Fix agent prompts keep the existing `build_fix_prompt()` output (dependency update or code scanning instructions). Append the MCP lifecycle call and wait instruction to the end of the generated prompt.

## TUI Integration

### Card badges

PR/alert cards with an active agent show a colored status badge:
- `[reviewing]` — yellow — agent is analyzing
- `[ready]` — green — findings ready, user should look
- `[idle]` — dim — agent waiting

Replaces the current binary "dispatched" indicator.

### Notifications

When `findings_ready` arrives via MCP, the card flashes using the existing `message_flash` mechanism.

### Key bindings (unchanged)

| Key | Action | Context |
|-----|--------|---------|
| `d` | Dispatch review agent | PR without active agent |
| `g` | Jump to agent tmux session | PR with active agent |
| `T` | Detach/kill tmux session | PR with active agent |
| `r` | Re-trigger review | PR with idle agent |

**`r` key change**: Remove manual refresh (`r` currently refreshes PR lists). The board auto-refreshes every 30s, making manual refresh redundant. `r` now exclusively triggers re-review: sends `/review-pr {number}` via tmux `send-keys` (same pattern as `send_message` for tasks) and sets `agent_status = Reviewing`. Only shown when the selected PR has an idle agent.

### Status bar hints

Update `review_action_hints()` and `bot_action_hints()` to show context-aware hints based on agent status. The existing `push_hint_spans()` embeds single-char keys into labels when the key matches the first letter (e.g. `push_hint("r", "refresh")` → `[r]efresh`).

Hint sets by agent state:

- **No agent**: `[d] review  [p] open  ...`
- **Reviewing**: `[g]o to  [T] detach  ...` (no re-review while analyzing)
- **FindingsReady**: `[g]o to  [T] detach  ...` (card is flashing green)
- **Idle**: `[g]o to  [r]e-review  [T] detach  ...`

Similarly for `security_action_hints()` with fix-agent equivalents.

### Auto-cleanup

Existing PR merge detection (polling) clears `tmux_window`. Extend to also clear `agent_status`.

Existing `DetachTmux` (`T` key) clears `tmux_window`. Extend to also clear `agent_status`.

## Lifecycle

```
dispatch (d) → reviewing → findings_ready → idle
                                              ↓
                              re-review (r) → reviewing → ...

detach (T) or PR merged → cleared (no agent)
```

- **dispatch**: Creates worktree, spawns tmux, sets `agent_status = Reviewing`
- **findings_ready**: Agent calls MCP tool after `/review-pr` completes
- **idle**: Agent calls `update_review_status(status="idle")` when the user says they're done reviewing
- **re-review**: User presses `r`, TUI sends `/review-pr {number}` to tmux session, sets `agent_status = Reviewing`
- **detach/merge**: Clears all agent state (`tmux_window`, `worktree`, `agent_status`)
