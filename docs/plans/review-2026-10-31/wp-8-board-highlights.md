# Board Highlights

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Keep highlights on the same item across folds and reloads, and tidy the admin screen's service shortcuts, wording and tests.

## Context

This work package addresses findings B1–B5 from `docs/plans/2026-10-31-codebase-review.md`, plus the `A` key decision. B1 and B2 are the same mistake (a row index kept across a list change); add one sentence to `docs/conventions.md`: highlights that survive a reload are kept by id, not index.

## Findings

### 🟡 Medium — B1: `Z` on the Epics tab moves the cursor to another epic (`src/tui/update/board_filter.rs:136`, `:208`)

**Issue:** `handle_toggle_all_filter_epic_folds` only clamps the cursor (`clamp_epic_cursor`). Since 00ef8606 every epic starts folded, so the first `Z` unfolds all: with the cursor on the 4th root, the children of roots 1–3 appear above it and Space then hides a child of root 1. Folding back clamps a deep child to the last row instead of its folded ancestor. `board-layout.allium` (~1160) says the cursor stays on its row; the `capital_z_*` tests in `src/tui/tests/filter_overlay.rs` never assert the cursor. Also: `Z` during an active epic query clears unfolds invisibly.

**Fix:** Remember the cursor row's epic id; after the change, place the cursor on that id or its nearest visible ancestor. Make `Z` a no-op while an epic query is active (record in the spec). Test both.

### 🟡 Medium — B2: the admin screen loses the highlight after a write (`src/tui/update/admin.rs:87`)

**Issue:** `handle_admin_loaded` keeps only the workspace highlight by id. After `n` creates a workspace, the old row stays highlighted (`admin-surfaces.allium: CreateWorkspaceFromScreen` says the new one is), so the next `a` adds a member to the wrong workspace. Member, super-admin and unseen-item highlights are indexes; after a self-demotion with `m` the member index points into a neighbour's list and `x` targets an unrelated person. `AdminScreenRefreshesAfterAWrite` says highlights keep their place by id.

**Fix:** Keep the membership id, super-admin identity and unseen (kind, id) and look them up after reload; reset `member` when the workspace changes; after a successful `CreateWorkspace`, target the new workspace (by its returned id or unique name). Tests for each.

### 🟢 Low — B3: `PeopleService` shortcuts skip the permission check (`src/service/people.rs:54`, `:127`)

**Issue:** `set_super_admin` and `set_member` return success without calling the reducer when nothing would change, so a caller with no authority is told "X is a super-admin". These are data rules in `src/service/` (against CLAUDE.md's direction). `PeopleService::set_member` also rebuilds the admin snapshot three times per call.

**Fix:** Always call the reducer; let the module treat a repeat as a no-op. Build the snapshot once.

### 🟢 Low — B4: small MCP and spec mismatches (`src/mcp/handlers/admin.rs::format_workspaces`, `src/store/queries/people.rs::admin_snapshot`, `src/runtime/people.rs::write_admin`, `src/tui/messages/admin.rs:13`, `src/mcp/handlers/epics.rs::unknown_workspace`)

**Issue:** `list_workspaces` prints membership ids no tool accepts (mcp-tool-design #7). `admin_snapshot` orders unseen items by (kind, id), not the spec's (workspace, kind, id). The add-member status says "Added {name}", not "Added <name> to <workspace>". `AdminMessage::Open` and `handle_admin_open` docs say `W`; the key is `A`. `create_epic` tells a super-admin "unknown workspace_id N" for a workspace `list_workspaces` just showed.

**Fix:** Drop the membership id; sort by (workspace, kind, id); add the workspace name; fix the two comments; say "you are not a member of workspace N" when the caller can see it.

### 🟢 Low — B5: test quality (`src/tui/ui/kanban/popups/admin.rs::tests::an_unseen_row_shows_no_title`, f6c722a9, 1f916aa7)

**Issue:** The admin test only checks a heading and cannot fail. f6c722a9 waits in an unbounded `woken.changed()` loop; 1f916aa7 uses a 10 s bound; conformance files each define their own `wait_for!`.

**Fix:** Assert the rendered row's exact text (or delete the test). One bounded wait helper in `tests/spacetime_module`, used by both.

## Decisions

### `A` now opens the admin screen (`src/keybindings/table.rs`, `docs/specs/keybindings.allium`)

**Question:** `A` was the only-active toggle (heavy use per `board-layout.allium` ~933) and now opens a modal admin screen where `m` and `c` act at once. Keep it?

**Options:**
- (a) Keep `A`; record the reuse in `keybindings.allium`. No code change.
- (b) Move the admin screen to another key (e.g. under a leader or `Ctrl-A`). Costs a key-table change, spec and help text.

Ask the user with `AskUserQuestion` before implementing this decision. Do not pick an option yourself. If the user does not answer, leave it unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `src/tui/update/board_filter.rs` | Cursor by id across `Z`; `Z` no-op during query |
| `src/tui/tests/filter_overlay.rs` | Cursor assertions |
| `docs/specs/board-layout.allium` | `Z` during a query |
| `src/tui/update/admin.rs`, `src/tui/messages/admin.rs` | Highlights by id; new-workspace target; comments |
| `src/tui/tests/admin.rs` | Highlight tests |
| `src/service/people.rs` | Always call the reducer; one snapshot |
| `src/mcp/handlers/admin.rs`, `src/mcp/handlers/epics.rs` | Output and refusal wording |
| `src/store/queries/people.rs` | Unseen order |
| `src/runtime/people.rs` | Status wording |
| `src/tui/ui/kanban/popups/admin.rs` | Fix or drop the test |
| `tests/spacetime_module/` | Shared bounded wait helper |
| `docs/conventions.md` | Highlights by id |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass, no `skipping:` lines
- [ ] Snapshot tests reviewed if any change (`docs/testing.md`)
