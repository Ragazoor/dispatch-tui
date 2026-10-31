# Codebase review — 2026-10-31

Task #98654. Follows `docs/plans/2026-10-28-codebase-review.md` (task #98508). This pass checks every 2026-10-28 finding against today's tree and reviews the 35 commits since that report (00ef35ab..f6c722a9): the seven follow-up work packages, the removal of repo sync, and the workspaces feature (workspace and visibility columns, sight checks on writes, per-caller views, the admin screen and its MCP tools, workspace-scoped learnings, moving a root epic between workspaces). Three read-only reviewers plus inline checks of the High and every Medium. Paths are relative to the worktree root.

**What was run.** `cargo test --no-fail-fast` and `cargo clippy --all-targets -- -D warnings` on this host. Results are in "Test and lint run" below. Coverage was not re-measured; the last figure is 92.17% (2026-10-15, `docs/testing.md`).

**Confidence key.** *Verified*: re-read in the code. *Plausible*: the path exists but depends on environment or timing.

## 1. Executive summary

- **The 2026-10-28 work packages landed well.** 19 of 23 findings are Fixed, P2 is moot (repo sync is gone), and the long-open `PaneId` newtype and S1 relink are done. The trust model is now written down in `docs/specs/workspaces.allium`.
- **One High: the `seed_*` reducers check no caller.** Any connected board can make itself a super-admin, join any workspace as admin, or rewrite a private epic to public. This undoes the workspace permission model.
- **The workspace rules have holes around moves.** Re-parenting an epic or a task under your own private epic hides other people's work with no visibility authority; repo-group routing and flatten ignore visibility; the client still names a task's `owner`.
- **Two partial-write and two rule-gap regressions in the fixes:** `update_epic` makes three writes, repo-group relinks keep the unseen mark (N1's sibling), and the P1 fix overrides `core.sshCommand`.
- **Board side:** `Z` on the Epics tab and the admin screen's reload both move the highlight to a different row, against the spec.

## 2. Findings

### 2.1 Workspaces: authority and sight in the module

Checked clean: `RULES_VERSION` and `BOARD_RULES_VERSION` move together on every module commit (now 23); view bodies in `spacetime/module/src/views.rs` are shared with the twin through `StoreReads`; `move_epic_to_workspace` matches its twin in order of checks, wording and cascade; delete paths check sight of the whole subtree before naming tasks.

**V1. High — the seed reducers have no authorisation.** `spacetime/module/src/seed.rs::seed_user_profiles` (~438), `::seed_workspaces` (~459), `::seed_memberships` (~474), and also `seed_epics`, `seed_work_packages`, `seed_learnings`, `seed_subscriptions`. *Verified.* None checks `visibility.rs::is_operator` or the caller. Scenario: any board identity calls `seed_user_profiles([{identity: me, is_super_admin: true}])` and is a super-admin; `seed_memberships` makes it admin of any workspace; `seed_epics` flips another person's private epic to public or rewrites `created_by`. The spec allows it too: `spacetime-seed.allium` declares `actor Operator { identified_by: core/Host where id != "" }`. The restore path calls these from `src/spacetime/cli_store.rs` (~421). Fix: decide which identity restores (the publisher token, so `is_operator` holds, or a super-admin), gate every `seed_*` on it, tighten `Operator` in the spec, and add a refusal test per reducer.

**V2. Medium — re-parenting under your own private epic hides others' work.** `spacetime/module/src/visibility.rs::check_epic_move` (~190), called from `tasks_epics.rs::patch_epic`; twin `src/sync/memory_caller/visibility.rs::Tables::check_epic_move`. *Verified.* The only checks are that the destination is visible and in the same workspace. Scenario: member M makes private epic P, then patches Y's public epic E (with running tasks) to `parent_epic_id = P`. E's creator and the hosts running its tasks lose sight; their hooks are answered "does not exist". E stays `visibility = "public"`, so `unseen_items_of` never lists it and the super-admin cannot recover it. Fix (spec first, `EpicStaysInItsWorkspace`): a move under a private destination needs the authority `set_epic_visibility` needs, and a subtree with nothing of anyone else's. Mirror and add a conformance case.

**V3. Medium — the task version of V2, and a client-chosen owner.** `tasks_epics.rs::move_task_row` (~531) and `::update_task` (~587); twin `apply_set_task_epic`, `apply_update_task`. *Verified.* (a) Any task in a public epic can be pulled into a private epic by its holder. (b) A move to the main board (`epic_id = 0`) takes `owner` from the caller's argument: `set_task_epic(T, 0, "<anyone>")` takes a teammate's task, plants it on someone else, or strands it with `""`. The spec says it lands on the caller's own board. `respawn_phoenix_successor` and `create_task` with epic 0 also trust the client's owner. Fix: stamp `owner = caller_identity` (operator exempt) and apply V2's rule. Fold into V2.

**V4. Medium — repo-group routing and flatten ignore visibility.** `spacetime/module/src/grouping.rs::find_or_create_repo_group` (~44), `::route_to_repo_group` (~23), `::flatten_epic` (~200); twin `src/sync/memory_caller/grouping.rs`. *Verified.* A repo-group sub-epic is found by title and origin only, and its creator may make it private. Scenario: later `create_task` calls aimed at the public root land in that private sub-epic and leave the creator's own view, so the caller never sees the row it waits for. `flatten_epic` on the root moves the private sub-epic's tasks into the public root and deletes it without `check_subtree_all_seen`. Fix: refuse `set_epic_visibility` on `origin == repo-group` epics (simplest), or skip unseen sub-epics in routing and refuse flatten over them. Fold into V2.

**V5. Medium — an orphaned private sub-epic under a private parent is never listed.** `visibility.rs::unseen_items_of`, the `under_private` filter. *Verified.* It excludes every private epic under a private ancestor, even when that ancestor is not itself listed. Scenario: Y makes private S under X's public P; X makes P private; Y leaves. S is seen by nobody, not listed, and blocks deleting P and the workspace. Fix: exclude only when the nearest private ancestor is itself listed. The spec also contradicts itself here (nearest-holder rule vs D5 "a private epic hides its whole subtree"); pick one in `core.allium` first.

**V6. Medium (plausible) — the v17 migration is skipped when a workspace exists.** `spacetime/module/src/workspace.rs::migrate_store_to_workspaces` (~343). The early return on `workspaces().iter().next().is_some()` skips the backfill even when `previous == 16`, and v16 shipped `create_workspace`. Scenario: a super-admin made a workspace under v16; after publishing v17 every epic and package stays at `workspace_id 0`, and every write to them is refused as missing. Fix: when `previous < 17`, backfill rows with `workspace_id == 0` into "Default" (create it if absent) whether or not other workspaces exist.

**V7. Low — the client still names identities in some writes.** `config.rs::subscribe_to_epic` and `unseen.rs::mark_epic_seen` take a free `subscriber`; `visibility.rs::settle_new_epic` keeps a client `created_by` for the operator path, and `check_new_package` never stamps it. *Verified.* Views filter the rows, so nothing leaks, but a caller can clear another person's unseen marks or plant a private sub-epic attributed to Y, which then blocks `move_epic_to_workspace`. Fix: use `caller_identity` (operator exempt). Fold into V1.

**V8. Low — cross-workspace learning rescope keeps the old `workspace_id`.** `spacetime/module/src/learning.rs::rescope_epic_learnings`, `::rescope_learnings`. *Verified.* Only `scope_ref` moves; the learning becomes visible only to people in both workspaces. Fix: set `workspace_id` from the target epic, as `move_epic_to_workspace` does, or refuse. Fold into V2.

**V9. Low — unlinking checks no sight.** `spacetime/module/src/packages.rs::update_work_package_with_links`, the unlink list. *Verified.* A member who sees a public package can remove its link to an item they cannot see. `WritesRequireSight` names links. Fix: treat an unseen item as not linked. Fold into V2.

**V10. Medium — conformance gaps for the visibility pairs.** *Verified.* Only `tests/memory_caller_conformance/move_epic.rs` and `learning_placement.rs` run with `with_workspace_rules()`. No module-vs-twin case covers `set_epic_visibility`, `set_work_package_visibility`, `super_admin_delete_unseen_item`, the unseen-is-missing refusals, the creator stamp, 06874206's refusal, or the views. `src/sync/memory_caller/views.rs`'s module doc claims a live comparison that does not exist. The twin's `Tables::watchers_of` returns `Vec::new()` while the module reads both watcher indexes. Fix: add the pair tests (they will also pin V2–V5), implement `watchers_of`, and correct the doc.

### 2.2 Sync side of the views

**Y1. Medium (plausible, latent) — a row served by two views is erased when it leaves one.** `src/sync/rows.rs::SharedRows::retire_task`, `::retire_epic`; `src/sync/sdk_connector/wiring.rs` (`wire_view!`). Retire removes the row when the held row equals the retracted one, without knowing which view sent it. Once both `own_epics` and `covered_epics` (or the task pair) feed one map, as task #98691 plans, unfollowing an epic drops an epic the caller created inside it. Fix: per-view id sets or reference counts before #98691 lands; add the case to `sync.allium` `AViewUpdateNeverErasesTheRow`.

### 2.3 Fix commits for the 2026-10-28 findings

**R1. Medium — bounded git calls override the user's ssh command.** `src/process.rs::git_ssh_command` (~194) and `run_bounded`. *Verified.* Only `GIT_SSH_COMMAND` is read before appending `-o BatchMode=yes`; setting it overrides `GIT_SSH` and `core.sshCommand`. Scenario: an operator with `core.sshCommand = ssh -i ~/.ssh/work_key` now fails every bounded fetch, `ls-remote` and wrap-up pull. `docs/conventions.md` says the existing command is kept. Fix: leave `GIT_SSH_COMMAND` alone and start the child in a new session (`setsid`) so ssh cannot open `/dev/tty`; or resolve the effective command (`GIT_SSH_COMMAND`, `GIT_SSH`, `git config core.sshCommand`) before appending.

**R2. Medium — `EpicService::update_epic` makes three writes.** `src/service/epics.rs::update_epic` (~417-427): `patch_epic`, then `move_epic_to_workspace`, then `set_epic_visibility`. *Verified.* Same shape as S1. Scenario: a non-creator edits an epic's title and visibility in the editor; the title saves, the visibility is refused, the user sees an error and the store keeps the new title. Fix: one reducer (or `patch_epic` carrying workspace and visibility), mirrored with a conformance case.

**R3. Medium — repo-group relinks keep the unseen mark.** `spacetime/module/src/grouping.rs::relink_task` (~131), used by `reroute_on_repo_change` and `regroup_epic`; twin `src/sync/memory_caller/grouping.rs`. *Verified.* No `clear_unseen_for_task`, against `UnseenMarkEndsWithTheTaskLeavingItsEpicOrFinishing`. N1 fixed the feed path only. Scenario: a marked task's repo is edited and it moves to another repo sub-epic still marked. Fix: clear marks in `relink_task`, module and twin; conformance case; rebuild `module.wasm`.

**R4. Low (plausible) — G1's hazard moved to tree exit, and its guard test was deleted.** `src/agent_tree/run.rs::DiffPaneTeardown::drop` → `tear_down_diff_pane` → `reconcile_diff_pane(false)`. If `diff_pane.rs::mark_content` fails after `v` (it logs and swallows), quitting the tree kills the editor. a29c0c51 deleted `a_failed_content_lookup_is_an_error_rather_than_a_kill`. Fix: return the `set-option` error from `show_in_editor`; re-add the test.

**R5. Low (plausible) — S2 residue.** `src/service/tasks/crud.rs::update_task` still decides ClosePrOnDone from the cached prior row. `src/tui/commands/task.rs::PersistFields::from_task` forwards `completed_at` on every persist, so a Done→Done write can undo another board's reorder. Fix: have the module report the prior status from `update_task`; forward `completed_at` only on an explicit reorder.

**R6. Low — S3 residue.** `crud.rs::move_task_to_epic` checks `task_exists` and then calls `set_task_epic`, which answers `Ok` for a missing task. *Verified.* Fix: route through `update_task` with an empty patch, or reuse `refusal_as_not_found`.

**R7. Low — T1/T2 residue (test-only).** A dropped `TuiRuntime` aborts its tasks but never disconnects the SDK connection (`src/sync/sdk_connector/connect.rs` says dropping closes neither socket nor thread). `src/runtime/tests/live_store.rs::try_start` can still accept a sibling's store while its own child is starting. Fix: `impl Drop` that calls `disconnect()`; confirm the answering store by database identity.

**R8. Low — stale docs.** `src/git.rs` (~238) lists "measuring drift" as a caller of `ahead_behind`; the drift indicator is gone. `src/agent_tree/diff_viewer.rs` (~239-248, ~322) still talks about row order in the open set. *Verified.*

### 2.4 Board and MCP surfaces

Checked clean: every admin write goes through `PeopleService` to a reducer; no `unwrap`/`expect` outside tests in the new files; the lens removal left no dead code; every admin key has an MCP twin and vice versa; closed sets use `enum`; admin keys live in their own `board.admin` namespace.

**B1. Medium — `Z` on the Epics tab moves the cursor to another epic.** `src/tui/update/board_filter.rs::handle_toggle_all_filter_epic_folds` (~136), `::clamp_epic_cursor` (~208). *Verified.* The cursor is a row index and is only clamped. Since 00ef8606 every epic starts folded, so the first `Z` unfolds all. Scenario: cursor on the 4th root; `Z` inserts the children of roots 1–3 above it; Space now hides a child of root 1. The spec says "The cursor stays on the row it was on" (`board-layout.allium` ~1160). The `capital_z_*` tests never assert the cursor. Fix: remember the row's id, then find it or its nearest visible ancestor after the change. Also make `Z` a no-op while an epic query is active.

**B2. Medium — the admin screen loses the highlight after a write.** `src/tui/update/admin.rs::handle_admin_loaded` (~87), `::clamp_cursors`. *Verified.* After creating a workspace the old row stays highlighted (`CreateWorkspaceFromScreen` says the new one is), so the next `a` adds a member to the wrong workspace. Member, super-admin and unseen-item highlights are row indexes: after a self-demotion the member index points into a neighbour's list and `x` targets an unrelated person (the confirmation names them). `AdminScreenRefreshesAfterAWrite` says highlights keep their place by id. Fix: keep ids for all four lists; target the new workspace after a create; reset `member` when the workspace changes.

**B3. Low — `PeopleService` shortcuts skip the store's permission check.** `src/service/people.rs::set_super_admin` (~54), `::set_member` (~127). *Verified.* A repeat request returns success without calling the reducer, so a caller with no authority is told "X is a super-admin". These are also data rules in `src/service/`. Fix: always call the reducer and let it treat a repeat as a no-op.

**B4. Low — small MCP and spec mismatches.** *Verified.* `src/mcp/handlers/admin.rs::format_workspaces` prints membership ids no tool accepts (mcp-tool-design #7). `src/store/queries/people.rs::admin_snapshot` orders unseen items by (kind, id), not the spec's (workspace, kind, id). `src/runtime/people.rs::write_admin` says "Added {name}" without the workspace. `src/tui/messages/admin.rs` and `handle_admin_open` say `W` opens the screen; the key is `A`. `src/mcp/handlers/epics.rs::unknown_workspace` tells a super-admin "unknown workspace_id N" for a workspace `list_workspaces` just showed; say "you are not a member" instead. Fold into B2.

**B5. Low — test quality.** `src/tui/ui/kanban/popups/admin.rs::tests::an_unseen_row_shows_no_title` cannot fail. f6c722a9 waits in an unbounded `woken.changed()` loop; 1f916aa7 uses a 10 s bound; the conformance files each define their own `wait_for!`. Fix: one bounded helper in `tests/spacetime_module`; rewrite or delete the admin test. Fold into B2.

### 2.5 Test coverage

6,595 tests pass across 32 targets, 2 ignored, no `skipping:` lines (`tmux` and `spacetime` on `PATH`). Coverage not re-measured. The big gap is V10: the visibility rules are tested on the twin, but module-vs-twin agreement is pinned for only two reducers. Each finding above names the test it should add.

### 2.6 Complexity hotspots

Largest non-test, non-generated files: `src/tmux.rs` (1,688, +94 for `PaneId`), `src/keybindings/table.rs` (1,551), `src/dispatch/mock_sequence.rs` (1,531, test-support), `src/dispatch/worktree.rs` (1,499), `src/store/mod.rs` (1,335), `src/sync/rows.rs` (1,319, new to the list), `src/sync/sdk_connector/callers.rs` (1,280), `src/runtime/mod.rs` (1,258). New this week: `spacetime/module/src/visibility.rs` (681) and `views.rs` (630) and their twins (639, 235). `src/store/queries/people.rs::admin_snapshot` (~100 lines) scans every profile per member, and `PeopleService::set_member` rebuilds the snapshot three times per call. One `too_many_arguments` allow remains. Repo sync's removal cut ~2,500 lines.

### 2.7 CLAUDE.md improvements

1. **Any `epic_id` change ends unseen marks** — this is the second time a relink path missed it (N1, R3). One line in `docs/invariants.md` naming `clear_unseen_for_task` as the call every relink must make.
2. **Every reducer is callable by every client.** `docs/invariants.md` could say plainly that a reducer without a caller check is a public write, including `seed_*` (V1).
3. **Ids, not indexes, for highlights across reloads** — B1 and B2 are the same mistake; a sentence in `docs/conventions.md`.

## 3. Status of the 2026-10-28 findings

| # | Finding | Status | Evidence |
|---|---|---|---|
| S1 | Relink and patch are two writes | **Fixed** | 5a5209b7; `tasks_epics.rs::update_task`; same shape returns in R2 |
| S2 | Done decided from the cache | **Partly fixed** | module stamps `completed_at`; residue R5 |
| S3 | Service pre-checks | **Partly fixed** | `refusal_as_not_found`; residue R6 |
| N1 | Feed move keeps the unseen mark | **Fixed** | 2e523c8d; sibling gap R3 |
| N2 | Format characters in names | **Fixed** | `user_profile.rs::FORMAT_RANGES` |
| N3 | `resume_task` host gate | **Fixed** | 31599044, 54faffef |
| G1 | `v` runs the close path | **Fixed** | a29c0c51; residue R4 |
| G2–G5 | Agent-tree pane lifecycle | **Fixed** | `tmux::pane_roles_with_content`, `DiffPaneTeardown`, `RefreshSchedule` |
| G6 | Stale doc comments | **Mostly fixed** | residue R8 |
| P1 | Prompting remote hangs | **Fixed, regression** | 8aae4b80; R1 |
| P2 | Fast-forward vs sync race | **Moot** | a2f11560 removed repo sync |
| P3–P5 | Editor trim, header escape, CLI wording | **Fixed** | 8aae4b80, 42f060d9 |
| T1 | First board outlives drop | **Fixed** | 1c0bf2f4; residue R7 |
| T2 | Port race | **Partly fixed** | residue R7 |
| T3 | Previous-store gate in CI | **Fixed** | `ci.yml` `fetch-depth: 0` |
| — | `PaneId` newtype | **Fixed** | 0aed824c, 7de7c5db |
| W2 | Unowned repo paths | **Open (optional)** | `src/sync/rows.rs::repo_paths_saved_by` |
| — | Trust model | **Decided** | `docs/specs/workspaces.allium`; V1 is its enforcement gap |

## 4. Prioritised action items

### Magic wand (top 3 by impact)

1. **Gate the seed reducers** (V1). One check per reducer closes a hole that undoes every other workspace rule.
2. **One rule for moving work under a private epic** (V2–V4, spec first). Moves, task owner, repo groups and flatten are four doors into the same room.
3. **Module-vs-twin conformance for the visibility pairs** (V10). The twin runs every MCP and TUI test; without these cases V2–V5 and any fix to them can drift unseen.

### Quick wins

- R8 — two stale doc comments.
- B4 — membership ids, order, status wording, the `W` comment.
- R3 — one `clear_unseen_for_task` call in each of module and twin.

### Work packages

- **WP-seed-authorisation:** V1, V7 (`spacetime/module/src/seed.rs`, `config.rs`, `unseen.rs`, `visibility.rs`, `docs/specs/spacetime-seed.allium`, `src/spacetime/cli_store.rs`).
- **WP-private-moves:** V2, V3, V4, V8, V9 (`visibility.rs`, `tasks_epics.rs`, `grouping.rs`, `learning.rs`, `packages.rs`, twins, `workspaces.allium`).
- **WP-unseen-listing-and-migration:** V5, V6 (`visibility.rs::unseen_items_of`, `workspace.rs`, `core.allium`).
- **WP-visibility-conformance:** V10, R3 (`tests/memory_caller_conformance/`, `src/sync/memory_caller/`, `grouping.rs`).
- **WP-view-retire:** Y1 (`src/sync/rows.rs`, `sdk_connector/wiring.rs`, `sync.allium`) — before #98691.
- **WP-epic-write-and-task-residue:** R2, R5, R6 (`src/service/epics.rs`, `src/service/tasks/crud.rs`, module reducer and twin).
- **WP-git-ssh:** R1 (`src/process.rs`, `docs/conventions.md`).
- **WP-board-highlights:** B1, B2, B3, B4, B5 (`src/tui/update/board_filter.rs`, `src/tui/update/admin.rs`, `src/service/people.rs`, `src/mcp/handlers/admin.rs`).
- **WP-cleanups:** R4, R7, R8 (`src/agent_tree/`, `src/runtime/tests/`, `src/sync/sdk_connector/`, `src/git.rs`).

### Decisions needed

- V1: which identity runs a restore (publisher token vs super-admin).
- V5: the spec's nearest-holder rule vs "a private epic hides its whole subtree".
- `A` now opens the admin screen; it was the only-active toggle with heavy use. Keep, or pick another key and record it in `keybindings.allium`.
- `rate_learning`'s `not_applicable` widened accepted input without a `RULES_VERSION` bump; confirm the bump policy covers new accepted values.
- W2 (optional): adopt unowned repo paths on prune.

## Test and lint run

`cargo test --no-fail-fast`: 32 targets, 6,595 passed, 0 failed, 2 ignored, no `skipping:` lines. `cargo clippy --all-targets -- -D warnings`: clean.
