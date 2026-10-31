# Admin Screen and Board Filter

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make the admin screen show its refusals, keep highlights on the right row, and let the store answer membership questions.

## Context

Findings B1–B7 and the admin part of R6 from `docs/plans/2026-10-31-codebase-review.md`. Specs: `docs/specs/admin-surfaces.allium`, `workspaces.allium`, `board-layout.allium`, `keybindings.allium`. B6 items say "fix the code or the spec"; prefer the code unless noted.

## Findings

### 🟡 Medium — B1: the admin screen hides every refusal (`src/tui/ui/kanban/mod.rs::render`)

**Issue:** `render_error_popup` is drawn before `render_admin_overlay`, which clears its 80% area. Refusals (delete a non-empty workspace, unknown name, last super-admin) are invisible, and the next key is swallowed as "dismiss". `admin-surfaces.allium` says every refusal shows.

**Fix:** draw the error popup last; add a render (snapshot) test of a refusal over the admin screen.

### 🟢 Low/Medium — B2: the board decides membership before asking the store (`src/mcp/handlers/epics.rs::unknown_workspace`; `src/store/queries/people.rs::PeopleStore::admin_snapshot`)

**Issue:** before the identity settles or a membership row arrives, `my_workspaces` is empty, so `create_epic`/`create_work_package` and `list_workspaces` say "You belong to no workspace" though the store would accept. `NameWorkspaceAndVisibilityViaMcp`: every refusal is the store's.

**Fix:** send the create; use the local list only to decorate the store's refusal; say "identity not settled yet" when `me` is `None`.

### 🟢 Low/Medium — B3: admin cursors keep position, not id (`src/tui/update/admin.rs::App::handle_admin_loaded`)

**Issue:** only the workspace cursor is restored by id; members, super-admins and unseen keep their index while lists re-sort, so after granting "Aaron" the highlight sits on someone else and `v` targets them. A new workspace is not highlighted. Spec: `AdminScreenRefreshesAfterAWrite`, `CreateWorkspaceFromScreen`.

**Fix:** restore each pane by its key (membership id, identity, (kind, id)); after create, select the workspace by name.

### 🟢 Low — B4: `Z` on the Epics tab moves the cursor and ignores a query (`src/tui/update/board_filter.rs::App::handle_toggle_all_filter_epic_folds`)

**Issue:** `clamp_epic_cursor` keeps the index; since 00ef8606 the first `Z` unfolds all and the cursor lands elsewhere. With a title query, `Z` clears `open_epics`.

**Fix:** remember the cursor's `EpicId` and find it again; make `Z` a no-op during a query.

### 🟢 Low — B5: `set_super_admin`/`set_member` skip the store on a cached no-op (`src/service/people.rs::PeopleService::set_super_admin`, `::set_member`)

**Issue:** a non-super-admin "granting" an existing super-admin gets success unasked; a revoke can be skipped against a stale cache.

**Fix:** always call the reducer; let the module treat a no-op as success.

### 🟢 Low — B6: small spec/code mismatches (`src/store/queries/people.rs::admin_snapshot`; `src/models/people.rs::resolve_person`; admin status line)

**Issue:** unseen items ordered (kind, id), spec says workspace, kind, id; `resolve_person` says "ambiguous" where `PersonResolution` resolves by name (the code is arguably safer — fix the spec there); status reads "Added X", spec "Added <name> to <workspace>".

**Fix:** sort by workspace first; amend `PersonResolution`; fix the status text.

### 🟢 Low — B7 (latent): `SharedRows::retire_task` leaves undecodable rows counted (`src/sync/rows.rs::SharedRows::retire_task`)

**Issue:** when the departing row does not decode it returns early and never clears `undecodable_tasks`. Matters once tasks sit behind a view (#98691).

**Fix:** clear the entry when no decoded row remains for that id; add a unit test.

### 🟢 Low — R6 (admin part): stale `W` comments (`src/tui/update/admin.rs::handle_admin_open`, `src/tui/messages/admin.rs::AdminMessage::Open`)

**Fix:** say `A`.

Minor, fix in passing: the admin overlay has no scrolling, so long member/unseen lists push the highlight out of view; `rate_learning_not_applicable_leaves_the_score_alone` compares `updated_at` at millisecond resolution and may pass vacuously — use the mock clock.

## Changes

| File | Change |
|------|--------|
| `src/tui/ui/kanban/mod.rs` | Error popup last |
| `src/tui/update/admin.rs`, `src/tui/messages/admin.rs` | Cursor restore by key; comments |
| `src/tui/update/board_filter.rs` | Cursor by `EpicId`; `Z` no-op under query |
| `src/mcp/handlers/epics.rs`, `src/store/queries/people.rs` | Store decides membership; unseen order |
| `src/service/people.rs` | Always call the reducer |
| `src/models/people.rs`, `docs/specs/admin-surfaces.allium` | `PersonResolution` alignment |
| `src/sync/rows.rs` | `retire_task` cleanup |
| `src/tui/tests/admin.rs`, snapshots | Refusal render test, cursor tests |

## Verification

- [ ] `cargo test --no-fail-fast` — all pass; review new snapshots (`docs/testing.md` snapshot workflow)
- [ ] New test: a refusal on the admin screen is visible in the rendered frame
- [ ] `cargo clippy --all-targets -- -D warnings` clean
