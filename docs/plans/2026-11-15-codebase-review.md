# Codebase review — 2026-11-15

Task #99001. Follows `docs/plans/2026-11-05-codebase-review.md` (task #98918).

**Range.** `513f89df..d00aab72` on `main`, 233 commits (231 without merges; 180 touch code, tests, scripts or the plugin). Main themes: the 2026-11-05 work packages (epic #69690: board window checks, feed teardown aim and retries, loss of sight ends claims and marks, repo-path host authority, runtime editor and drain, bookmark MCP, process and tmux residues, graduated checks, the carried feed-runner items); soft delete into archived task records (`873287be`); refusing unfit sub_status inserts (`6962bc38`); removing the learning task scope (`209339d7`); agents named at task creation (`0171c8d7`); host.json locking and identity pairing (`e6418b80`, `f373e950`, `311b7afe`); batch and epic delete pre-checks (`d6215146`, `6d581921`); background-work waits in the hooks (`7a772517`); kept feed tasks (`37f6b697`); the configurable review command and `/preference-review` (`7168416e`, `52193082`); the compact epic tree and multi-move (`f5bc18d0`, `bf097886`); an Allium gate in CI (`1ade5bd3`); and many spec-only weed passes. The next run starts at `d00aab72`.

**How.** Fifth run of `.claude/skills/repo-health-review`. Six read-only reviewers, every code commit assigned to exactly one by hash: (A) status and regression audit of the module and sync fixes; (B) the same for runtime, process, tmux, feed and check-script fixes; (C1) defect hunt over the new module, store and sync features; (C2) defect hunt over feeds, review command, hooks, prompts and skills; (D) defect hunt over host identity, delete, TUI, runtime, startup and agent tree; (E) the fresh area, `src/store/` (plus `src/spacetime/managed_store.rs` and `src/runtime/store_target.rs`, where the store lifecycle lives). I re-read V1, V2, V3 and K1 in the code, ran the gates, and mutation-tested the graduated checks myself after the gate run.

**Gates (on `d00aab72`, `./scripts/verify.sh`, the repo's verify command; rustc 1.99.1 Fedora, equal to the pin).**
- `scripts/gates.sh --check` (toolchain, fmt, clippy `-D warnings`, every `check-*.sh` and its self-test, the module tests, `check-allium`: 42 specs, no errors, 16 baselined warnings): **green**.
- `cargo test --no-fail-fast`: **red** — 1 failure of 8,277: `memory_caller_conformance::schedules::memory_caller_keeps_schedules_like_the_real_module` (`left: TaskId(4096)`, `right: TaskId(2)`). Passes 4 of 4 runs alone; a load-dependent flake (S1). No `skipping` lines.
- Coverage not re-measured.

**Confidence key.** *Verified*: I re-read it in the code. *Verified by the reviewer*: a reviewer re-read it at `d00aab72`, I did not. *Plausible*: the path exists but the outcome depends on environment or timing.

## 1. Executive summary

- **The 2026-11-05 work packages landed well: 29 of 38 items closed**, 8 partly, 1 open (P5, a user decision). All three carried standing items (feed runner EOF, blocking git in `sync_emission`, `apply_update_epic`) are closed.
- **Two Highs, both "the fix guarded one door of two".** `respawn_phoenix_successor` is a public reducer that inserts a client-built task with no aim check and no identity stamp — the sibling A1's fix missed (V1). The verify-command fan-out lets anyone write the command other people's agents run, through a client-supplied GitHub path — the A3 fix checked only the named row (V2).
- **One-time migrations do not cover data that comes back through restore or import.** Old task-scoped learnings return seen by the whole workspace (K1); old snapshots with an unfit sub_status are refused half-way through a restore (K2).
- **`main` fails its verify command intermittently** (S1): the real store's ids jump to 4096 under load; e6253a0a stopped comparing work-package ids across stores but left task, epic and learning ids in nine other places.
- **The fresh area (`src/store/` and the managed store) is thin and mostly sound**; the defects are in the lifecycle around it: any caller can prune everyone's usage events (V3), a health restart can report success while the old, broken store keeps answering (M1), and several store commands still run without a timeout (M2).

## 2. Status of the 2026-11-05 findings

| # | Finding | Status | Where |
|---|---|---|---|
| C3 | Hook tests inherit the agent's `DISPATCH_TASK_ID` | Fixed | 4722c885, 7635ffa6 `tests/common/script_command.rs::plugin_script_command` (mutation-tested: 14 tests fail without the strip) |
| A1 | `create_task` aims another host | **Partly fixed** | 637732a1 refuses a host another person owns on both create reducers and the twin; `respawn_phoenix_successor` left open (V1); an unregistered host id is not judged |
| A2 | Detach reopens M10/M12 | **Partly fixed** | 69053de3 every task move releases claims and marks; the strand refusal is on detach only (L1) |
| A3 | Repo-path reducers trust the host | **Partly fixed** | c2ec77ec `require_acting_for` on all four; the fan-out reaches other people's rows (V2) |
| A4 | No upgrade path for per-host rows | **Partly fixed** | c2ec77ec `set_verify` checks this host's rows; one-time `unify_repo_verify_commands`; the board still never re-saves its paths, so legacy rows keep `github_path = ""` |
| A5 | Failed report strands a request | Fixed | 637732a1 `forget` on a refused claim |
| A6 | Setter-claims version moved | Fixed | 637732a1 back to 37, pinned by a test |
| A7 | Verify command by path alone | **Partly fixed** | c2ec77ec own row first; the fan-out's path-equality fallback still reaches same-path rows on other hosts (V2) |
| A8 | Detach keeps package links and learnings | Fixed / Moot | 69053de3 (links); 209339d7 removed task-scoped learnings |
| B1 | Marks survive loss of sight | Fixed | 69053de3 `on_sight_lost` / `on_task_sight_lost` on every path; spec settled ("losing sight ends the row") |
| B2 | `update_tasks` + bookmark is two calls | Fixed | afea3b42 |
| B3 | Stray `url_type` dropped | Fixed | afea3b42 |
| B4 | Bookmark conformance gaps | **Partly fixed** | afea3b42; marks after `move_epic_to_workspace`, `batch_delete`, `delete_epic`, member removal still uncovered (L2) |
| B5 | Mark scan; operator writes bookmarks | Fixed | afea3b42 |
| W1 | Boards crash other machines' agents | Fixed | dec8e686 host filter in `tick_window_checks`, `tick_sub_status`, `handle_window_gone`; spec `DetectCrashedAgent` |
| W2 | Failed pane query reads as closed | Fixed | dec8e686 `pane_exists` returns `Result` |
| W3 | Editor writes a stale snapshot | Fixed (store); residue | 3908be70 sends changed fields only; the local optimistic update still applies the snapshot (R6) |
| W4 | Refused edit still shown | Fixed | 3908be70 |
| W5 | Quit hangs on blocking threads | Fixed | 3908be70 |
| W6 | Tick refresh reads fewer tables | Fixed | 3908be70 one `read_board` |
| W7 | Two boards in one tmux server | Moot (decision) | dec8e686 focus by pane id; d87b938d specs one board per server |
| W8 | Signal skips teardown; retire kills pinned agent | **Partly fixed** | d87b938d returns pinned agents before the retire kill; the home tag it adds is never cleared (R4) |
| W9 | Runtime partial writes | Fixed | 74162444 one reducer for the group toggle; 3908be70 |
| W10 | Blocking work on the drain | Fixed | 74162444, 3908be70 |
| P1 | `pane_pid` unbounded | Fixed | 44d72032 |
| P2 | `store_import` bind-and-drop | Fixed | af7b797d `listener_is_ours`, d10b8a5c retries; `listener_has_arg` is itself unbounded (M2) |
| P3 | `SSH_ASKPASS_REQUIRE` overridden | Fixed | 44d72032 |
| P4 | `claim_store_owner` unbounded | Fixed | 44d72032; its siblings are not (M2) |
| P5 | Toolchain pin follows the distro | **Open (user decision)** | e9ffea03 reworded the docs; this machine still has no rustup |
| P6 | Feed config warnings taint cycles | Moot | 44d72032 made them fatal |
| P7 | Rules no spec states | Fixed | e9ffea03 |
| P8 | Keep-awake reads only the tmux env | **Partly fixed** | aa9c6993 tries the GNOME helper with no desktop variable; `OnceLock` remains; no fallback (R7) |
| G6 | Single-write checker stops at first `#[cfg(test)]` | Fixed | 97393167, 1a24e84c (mutation-tested: caught); a one-line `#[cfg(test)] item;` still hides the next item (G9) |
| G7 | Single-write checker misses | Fixed, residues | 97393167; see G9 |
| G8 | `epic_id` guard misses | Fixed, residues | 97393167; see G10 |
| Standing: feed runner waits for stdout EOF | | Fixed | c49c91c4 waits on the child, 500 ms grace, kills the group |
| Standing: blocking git in `sync_emission` | | Fixed | c49c91c4 `spawn_blocking` |
| Standing: `apply_update_epic` discards outcomes | | Fixed | c49c91c4 names the refused part; the twin can still half-apply, and its comment says it cannot (Low, folded into L2) |

Yield: 29 of 38 closed (Fixed, Moot or decision taken; W3 and G6–G8 count as fixed with residues); 8 partly (A1, A2, A3, A4, A7, B4, W8, P8); P5 open as a user decision. A1, A3, W8 and P4 are closed or partly closed with regressions or missed siblings (V1, V2, R4, M2).

## 3. Findings

Every module finding with a twin counterpart must be mirrored in `src/sync/memory_caller/` and get a case in `tests/memory_caller_conformance/`.

### 3.1 Gates

**S1. Medium — the conformance tests compare generated ids across the real store and the twin, and the real store's ids can jump.** `tests/memory_caller_conformance/schedules.rs` (`create` closure, "both sides hand out the same task id"), and the same assertion in `dispatch_requests.rs`, `close_and_managed_epics.rs`, `pr_state.rs`, `session_requests.rs`; `scenario.rs` (epic, task and learning ids), `learning_placement.rs`. *Verified.* The verify run on `d00aab72` failed with `left: TaskId(4096)`, `right: TaskId(2)`; the test passes alone. SpacetimeDB's auto-inc sequences hand out a fresh block of 4096 after a reallocation; e6253a0a fixed exactly this for work-package ids on 2026-11-11 and said so in its message, but left the other id kinds. Every agent's verify command can go red for a reason it did not cause. Fix: compare by creation order or by a per-side id map, as e6253a0a did, in every conformance file; then graduate it — a check (or a shared `create_both` helper the files must use) that rejects `assert_eq!(real_id, mem_id)` on a generated id. The class has now appeared twice.

### 3.2 Authority

**V1. High — fix for A1 (637732a1) → `respawn_phoenix_successor` inserts a client-built task with no aim check and no identity stamp.** `spacetime/module/src/tasks_epics.rs::respawn_phoenix_successor`; twin `src/sync/memory_caller/agent_state.rs::apply_respawn_phoenix_successor`; client `src/store/queries/tasks.rs` (`.respawn_phoenix_successor(predecessor, row)`). *Verified.* The reducer checks only that the predecessor is visible and phoenix and that the epic may be entered, then calls `write_task` with the caller's row (`..successor`): no `check_new_task_aim`, no `stamp_new_task`. Scenario: Bob sees phoenix task P in Alice's feed epic and calls it with `host = alice-h1`, `worktree = /home/alice/proj/.worktrees/999-live`, `external_id = "x"`, `created_by = "alice"`. Alice's laptop ingests; `x` is absent; a teardown is owed on h1 — A1's live-worktree removal again, and `created_by` is forged. Fix: run `check_new_task_aim` and `stamp_new_task` on both sides, or make the reducer private now that the module respawns through `respawn_phoenix_successor_of`; conformance case. Note `move_paths.rs::TWIN_SITES` lists the twin site as "inserts the successor, no prior mark" — the guard knows the path but not its authority.

**V2. High — fix for A3/A7 (c2ec77ec) → the verify-command fan-out writes other people's rows.** `spacetime/module/src/config.rs::set_verify_command` → `rows_of_the_repository_of` → `is_sibling`; `save_repo_path` (client-supplied `github_path`; sibling fill through `rows_without_the_repository_command`). *Verified.* `require_acting_for` guards only the named row. Scenario: Bob calls `save_repo_path("/tmp/x", now, bob-host, "alice-org/repo")` and then `set_verify_command("/tmp/x", bob-host, "curl …|sh")`; the fan-out rewrites every row whose `github_path` is `alice-org/repo`, including Alice's. When either side's `github_path` is empty, `is_sibling` falls back to path equality, so same-path rows on other hosts are reached too. Every agent of Alice's repository then runs Bob's command as its verify step. Fix: limit the fan-out and the sibling fill to rows whose host the caller owns (each person's rows of a repository agree among themselves), or stop trusting a client `github_path`; twin and conformance case. The spec `core.allium: SavedRepoPath` ("every row of one repository holds the same value") needs the same scoping.

**V3. Medium — any caller sets the usage-retention cap and prunes everyone's events.** `spacetime/module/src/config.rs::record_usage_event` / `prune_usage_events`; `src/store/queries/usage.rs::record_usage_event_with_cap`; `src/store/mod.rs::UsageCap`. *Verified.* The module accepts any `cap > 0` and deletes every `usage_events` row with `id <= highest - cap`, across all people. `record_usage_event(row, 1)` from an old binary, a stray test pointed at the real store, or anyone, wipes all telemetry, which the keybinding analysis then reads as "unused". Each event (one per keypress) also scans the whole table in the reducer. The cap is a data rule kept on the board, against CLAUDE.md's store layering. Fix: a module constant (ignore or clamp the argument); prune by id range.

**V4. Low — the hook reducers trust the caller's clock and host.** `spacetime/module/src/agent_state.rs::expire_background_wait`, `::background_wait_cutoff`, `::try_record_stop` (7a772517). *Verified by the reviewer.* The cutoff derives from a client `now`, while the comment says "a board cannot shorten the wait"; none checks `require_acting_for` on the task's host, so any viewer can expire another person's wait or hold their task running for 30 minutes with `shells = 1`. No new power over `try_record_stop`'s zero-count path, but the comment is false. Fix: use `ctx.timestamp`; require the caller to act for `task.host`; correct the comment.

**V5. Low — a super-admin's unseen-item delete names a task inside a private sub-epic.** `spacetime/module/src/visibility.rs::super_admin_delete_unseen_item` (and the twin). *Verified by the reviewer.* `validate_epic_deletable` runs before `subtree_holds_a_seen_private_epic`, and since 2c2279ca its refusal names the lowest-id undone task. `delete_epic` orders the checks the other way for this reason. Fix: swap the order on both sides.

### 3.3 Loss of sight

**L1. Medium — fix for A2 (69053de3) → the "refuse while another person's host runs it" rule covers detach only.** `spacetime/module/src/visibility.rs::check_task_move` (non-detach branch), `set_epic_visibility_row`, `tasks_epics.rs::patch_epic_row` (re-parent), `visibility.rs::move_epic_row`, `workspace.rs::remove_member`. *Verified by the reviewer.* Alice moves Bob's running task into her private epic, or makes its epic private: `on_task_sight_lost` releases Bob's claim, but his reactor still holds a live agent and open session rows; `get_task` answers None, wrap-up fails as "task deleted" and PR polling stops (M12 again). `workspaces.allium` records the refusal for detach only. The operator bypass in `check_task_move` also skips it. Fix: decide in the spec whether these paths refuse or may strand a host (a decision for the user), then apply `task_detach_strands_a_host` over the moved subtree on every path; conformance arms.

**L2. Low — the loss-of-sight and bookmark conformance cover claims, not marks parity.** `tests/memory_caller_conformance/loss_of_sight.rs`, `bookmarks.rs`. *Verified by mutation*: removing the twin's mark drop in `on_task_sight_lost` passes all 38 conformance tests (a twin unit test, `feed_authority::a_detach_ends_the_claims_and_marks_it_strands`, catches it); a module-side omission would pass both. No "move into a private epic" arm; B4's paths (`move_epic_to_workspace`, `batch_delete`, `delete_epic`, member removal) still uncovered. The twin's `apply_update_epic` comment says it never half-applies, which is false in the drift case. Fix: compare the marks each identity can read on both sides after every loss-of-sight arm; add the arms; reword the comment.

### 3.4 Live data: restore, import and archive

**K1. Medium — fix for the task-scope removal (209339d7) → a restore or import of older data brings task-scoped learnings back, seen by the whole workspace.** `spacetime/module/src/seed.rs::seed_learnings` (raw insert/update, no scope check); `lib.rs::stamp_rules_version_on_connect` (the retire runs only when the store's own stamp is below `TASK_SCOPE_REMOVED_RULES_VERSION`); `visibility.rs::learning_seen_by` (`_ => true` for any scope other than `epic`). *Verified.* The first connection stamps an empty store at 53, then `seed_learnings` inserts `scope = "task"` rows; `restore.rs` and `import.rs` run no retire afterwards. A learning about a task in someone's private epic is then readable by every workspace member (or by everyone, with `workspace_id = 0` and no author). The board drops the rows as "unknown scope", but the store still serves them. Fix: `seed_learnings` applies the retire to each row (or refuses `task`); a restore test with a pre-48 snapshot.

**K2. Medium — fix for unfit sub_status (6962bc38) → old snapshots and stores can no longer be restored or imported, and the refusal comes half-way.** `spacetime/module/src/rules.rs::check_sub_status_write` reached from `seed.rs::seed_tasks` → `tasks_epics.rs::store_task` (`prior = None`); `src/spacetime/restore.rs::restore`; `src/spacetime/import.rs`. *Verified by the reviewer.* `reset_unfit_sub_statuses` cleans the live store once; a snapshot holding backlog/approved is refused by `seed_tasks` after `burn_id_sequences` and earlier tables have been written, against `NothingIsWrittenBeforeTheChecksPass`. Fix: normalise unfit pairs to `default_sub_status` while preparing the snapshot or import (as the migration does), or refuse before the burn and name the rows.

The class behind K1 and K2: a one-time migration gated on the store's rules stamp does not see rows that arrive later through a seed path. See §5.

**K3. Low — the operator's archive listing misses records.** `spacetime/module/src/archive.rs::archived_tasks_for`. *Verified by the reviewer.* `core.allium: ArchivedTask` says the operator sees every record and `archived_task_for` agrees, but the list reads only the caller's owner index and listed workspaces. `list_tasks(archived: true)` and `get_task` disagree. Fix: for the operator, iterate the table.

**K4. Low (decision) — archived records keep their audience after the epic goes private or moves.** `archive.rs::archived_task_visible_to`. *Verified by the reviewer.* `core.allium` freezes the audience on purpose; `ClaimsFollowSight` and the "a flip takes the tree from every other member" intent in `visibility.rs` point the other way. Old-workspace members keep reading archived titles and descriptions. Fix: the user picks which rule wins; then spec and code.

**K5. Low — soft-delete conformance covers one path.** `tests/memory_caller_conformance/agents.rs` compares an epic-less `delete_task` record only. Not compared: private-chain records (`workspace_id`, `private_holder`), `batch_delete`, `delete_epic_subtree`, the super-admin hatch, or reads by member, non-member, holder and operator. *Verified by the reviewer.* Fix: those cases, read back as two identities.

**K6. Low — stale citation of a removed rule.** `src/spacetime/import.rs::DroppedIds` and `docs/specs/spacetime-seed.allium: DependentsOfDroppedRowsAreDropped` cite `agents.allium: AgentNameIsNeverReused`, which 0171c8d7 removed; names are reusable now, so an archived record's "Agent: X (archived)" can name a live agent who holds X. *Verified by the reviewer.* Fix: update both citations; consider showing the archived agent with its task id.

### 3.5 Feeds and hooks

**J1. Medium — fix for kept feed tasks (37f6b697) → a kept task someone resumes is moved back to done every cycle, killing its agent.** `spacetime/module/src/feed.rs::left_feed_moves_to_done`, `::keep_left_feed_task`; twin `src/sync/memory_caller/feed.rs::keep_left_feed_task`. *Verified by the reviewer.* `left_feed` is cleared only when the item returns, and `left_feed_moves_to_done` answers "finishing" for any running or review task. A merged review's card sits in done; the operator resumes it to follow up; the next cycle patches it to done and writes a close row, killing the window mid-work. Moving the kept card to backlog deletes it next cycle — losing the record the rule keeps. `feeds.allium: KeepWorkedOnFeedTask`'s idempotence note assumes the task stays done. Fix: move only on the first absence (`!task.left_feed`), or clear `left_feed` on a status change by hand; spec first.

**J2. Low-Medium — the Stop hook can exit before recording the Stop.** `plugin/hooks/scripts/task-status-hook` (`Stop)` arm, 7a772517). *Verified by the reviewer* (script mechanics); payload shape *Plausible*. Under `set -euo pipefail`, `COUNTS=$(echo "$INPUT" | jq -r '[.background_tasks // [] | .[].type] …')` exits on any non-object element, before any `dispatch hook … stop`. No Stop means `stop_pending` stays false, and `expire_background_wait` requires it, so the task stays running until moved by hand. The fallback `dispatch hook "$ID" stop` also fires on any failure, not only on an old binary's usage error, and can flip a still-waiting task to review. Fix: `|| COUNTS=""` and `.type?`; fall back only on exit code 2; a hook-script test with an odd payload.

**J3. Low (plausible) — feed predict-then-verify reads a task that left the view as deleted.** `src/store/queries/mod.rs::confirm_removed`. *Verified by the reviewer* (path). A local feed task whose epic goes private or moves during the ingest round trip is reported removed, and `FeedCycle` tears down its worktree and window while the task exists. Fix: confirm absence by id through a sight-independent read, or skip teardown when the row is merely invisible.

### 3.6 Runtime, tmux and delete

**R1. Medium — fix for the split leftover window (3f5cd27c) → only the board's kill spares it; the reactor, wrap-up and cleanup kill it with the user's shell.** `src/service/tasks/session.rs::kill_row_window`, `src/service/tasks/wrap_up.rs::kill_session_window`, `src/dispatch/worktree.rs::teardown_task` (`kill_window_if_present`); only `src/runtime/split.rs` (`KillTmuxWindow`) goes through `dispatch::kill_task_window`. *Verified by the reviewer.* Pin task 7 with a shell split beside it; the agent calls `exit_session` (the common end of a pinned task); the reactor's plain `kill_window("task-7")` kills the shell and its scrollback. Fix: route every task-window kill through `kill_task_window` (or move the awaiting-tag check into `tmux::kill_window`); extend `split-pane.allium`'s leftover-window rule to the reactor paths. Same "sibling path" class as V1/V2.

**R2. Medium — RetryFresh blocks the command drain on an untimed `git worktree remove`.** `src/runtime/commands.rs::run_cleanup` (`await_teardown`) → `dispatch::teardown_task` → `src/dispatch/worktree.rs::remove_worktree_and_branch` → `git::git_in_unbounded`. *Verified by the reviewer.* Chain: 5e34bf8f (wait for the cleanup) → 10e90b63 / 69cca0cc (wait only for RetryFresh). The one wait left is on the one git call that is deliberately unbounded: a multi-GB `target/` or a hung mount freezes keys and drawing. Fix: re-dispatch from the cleanup's completion message instead of awaiting, or bound the wait and report.

**R3. Medium (plausible) — the delete pre-check can pass, then the teardown kills an agent started in between.** `src/runtime/tasks.rs::exec_batch_delete`, `src/runtime/epics.rs::exec_delete_epic`, `::exec_cleanup_queues`. *Verified by the reviewer* (path), race *Plausible*. Targets are taken at keypress; `pass_delete_guard` checks; teardowns start fire-and-forget before `batch_delete`/`delete_epic` re-checks in the reducer. Another board reopens and resumes a task in between: its new window is killed and its worktree removed, then the reducer refuses the delete and the row points at nothing. Fix: delete first and tear down from the archived record that 873287be now keeps, or reserve the ids in one reducer before tearing down.

**R4. Low-Medium — fix for W8 (d87b938d) → the pinned pane's home tag is never cleared.** `src/dispatch/split_panes.rs::tag_pinned_pane`, `src/startup/retire.rs::return_pinned_agents`. *Verified by the reviewer*; tmux option persistence across `respawn-pane -k` *Plausible*. After the pinned task loses its window, the respawned shell still carries `@dispatch_home_window=task-7`. If `task-7` exists again, retire's break-out is refused and the launch aborts as `BoardWindowSurvived`; if not, the relaunch creates a phantom `task-7` holding a shell. Fix: `set-option -p -u @dispatch_home_window` on respawn and on restore.

**R5. Low — delete teardown queues and pre-checks.** `src/runtime/tasks.rs::exec_cleanup_queues` groups by the raw `repo_path` string (`~/x` and `/home/u/x` run in parallel; single `Cleanup` commands and provisioning are not serialised against a batch queue); `::pass_delete_guard` makes one `check_deletable` call per id although the procedure takes vectors (N round trips on the drain, N snapshots, against tasks.allium's "checks the whole batch once"). *Verified by the reviewer.* Fix: key queues on the expanded path; one `check_deletable(task_ids, epic_ids)` call.

**R6. Low — fix for W3 (3908be70) → the editor's local update still applies the snapshot.** `src/runtime/editor.rs::finalize_task_edit` → `src/tui/update/system.rs::handle_task_edited`; `finalize_epic_edit` (`updated = epic`). *Verified by the reviewer.* The store gets only changed fields, but the board copy gets the open-time status, model and wrap-up mode (and clears `stop_pending`) until the next push. Fix: apply only the changed fields locally.

**R7. Low (plausible) — fix for P8 (aa9c6993) → with no desktop variable, the GNOME helper is chosen with no fallback.** `src/keep_awake.rs::hold_command`. An ssh-started tmux server with `gnome-session-inhibit` installed but no session: the helper is refused, `systemd-inhibit` (which worked before) is never tried, and `OnceLock` keeps the choice. Fix: fall back on refusal; drop the caching of a failed choice.

**R8. Low — `m` on a selection.** `src/tui/update/move_task.rs` → N × `MoveToEpic`, each with a full refresh; the selection is cleared before results arrive; epics in the selection are silently ignored. Per-task refusal matches `BatchMoveToEpic`. *Verified by the reviewer.* Fix: keep refused ids selected (as d6215146 does for delete); refresh once; say that epics were skipped.

### 3.7 Fresh area: `src/store/` and the managed store

`src/store/` itself is transport: each read answers from `SharedRows`, each write is one reducer call that passes `self.host`, and the authority checks sampled all live in the module. Host files are sound: a `flock` spans every read-modify-write of `host.json`, the mode is set before content, and `mint_if_absent` uses `persist_noclobber`. The defects are in the lifecycle code beside it.

**M1. Medium — a health-check restart can report success while the old, broken store keeps answering.** `src/spacetime/managed_store.rs::ManagedStore::check_health` → `SpacetimeManagedStore::start_managed_store`. *Verified by the reviewer* (code); outcome *Plausible*. A stop that times out still goes on to start (`startup.allium` allows it); the start loop probes the port before `try_wait`, so the old store answers while the new child has not yet failed to bind, and the start returns `Ok`. The board says "restarted", writes keep failing, and the cycle repeats every 10 s. `StoreSpawner::listener_is_ours` exists for this but only `store_import` calls it. Fix: require `listener_is_ours(address, data_dir)` before `Ok`; `RestartFailed` when the stop did not complete; spec amendment. Related, Low: `check_health` reads `stopped` once at entry and calls stop and start directly, not through `stop_once`, so an exit landing between them leaves a store nobody holds (harmless: the next launch adopts it); and the health check reads `<board data dir>/managed-store.log`, so a store adopted from another `--data-dir` is never seen failing.

**M2. Medium-Low — fix for P4 (44d72032) bounded one store command; its siblings still run without a timeout.** `RealProcessRunner::run` (plain `Command::output()`) under `SpacetimeCliPublisher::publish`, `SpacetimeManagedStore::cli_identity`, `listening_pids` (`ss`, `lsof`), `signal` (`kill`); raw `Command::output()` in `listener_has_arg` (af7b797d), which also fails closed — with `ss` missing every `store import --from` is "foreign". *Verified by the reviewer.* A store that accepts connections but stalls hangs bring-up at "Starting the local store…" — the case P4's comment names. Fix: `run_with_timeout` everywhere (generous for publish, mapped to `PublishFailed`); "cannot tell" distinct from "foreign". Graduate: a `clippy::disallowed_methods` entry for `std::process::Command::output` outside `src/process.rs` (this is the third timeout finding: H3, P1/P4, now M2).

**M3. Low-Medium (plausible) — the shared module file and hash record are not written atomically.** `SpacetimeManagedStore::publish_embedded_module` (`fs::write` of `dispatch_module.wasm` at a fixed shared path, then publish from it); `FileModuleHashRecord::record_module_hash` (`fs::write`). Two overlapping launches (board plus import, or a main build plus a worktree build) can publish truncated bytes — and the Board abort then stops the shared store under the other launch — or record a hash that misdescribes the store. Fix: per-process temp file for the wasm; `atomic_write` for the hash; an `flock` across publish-and-record.

**M4. Low (plausible) — the stop signals single pids with SIGTERM only.** `SpacetimeManagedStore::stop`, `stop_spawned_store`. The spawn sets `process_group(0)`, but the stop never signals the group or escalates to SIGKILL. If `spacetime start` forks rather than execs, the server is orphaned on the port. Fix: `kill -TERM -<pgid>`, then SIGKILL at the deadline.

**M5. Low — small store-layer drift.** *Verified by the reviewer.* `src/store/queries/epics.rs::create_epic_configured` returns the row as sent, but `visibility.rs::settle_new_epic` rewrites `workspace_id`, `created_by` and `feed_command_set_by` (the MCP reply can name the wrong workspace). `queries/settings.rs::refuse_identity_key` refuses identity keys on the board only; the module's `save_setting` accepts them (spec-aligned, but a data rule on the board). `src/store/mod.rs` docs still talk about SQLite, "migration v35" and cycle detection in the service layer; `SessionRequestStore` claims every call names this host. `src/atomic_write.rs::write_with` / `create_new_bytes` do not fsync the parent directory after the rename (a power loss right after identity adoption can lose the credential). Fix: read the row back; move the key refusal to the module; fix the docs; sync the directory.

### 3.8 Skills

**N1. Low — preference-review skill and spec disagree after 7c71ae8a.** `plugin/skills/preference-review/SKILL.md` Step 4 ("split them by file across agents") vs `docs/specs/preference-review.allium` (`single_verifier_max_findings = 8`); Step 6 dropped the voice rules the spec's Present guidance still requires; the skill no longer says where `owner`/`repo` come from for a bare PR number, which Steps 1, 2 and 7 need. *Verified by the reviewer.* Fix: restore the three sentences, or change the spec.

**N2. Low (plausible) — the retro skill's description is not strict YAML.** `plugin/skills/retro/SKILL.md` (43bf57cb): an unquoted plain scalar containing `task: to finish`. `tests/repo_skills.rs::frontmatter_value` splits by line prefix, so it passes. Fix: quote it; parse frontmatter with a YAML parser in the gate.

### 3.9 The graduated checks (audited by mutation)

Mutation results (each re-introduced, run, then reverted; tree clean after): the G6 case (two writes below a mid-file `#[cfg(test)]` item in `src/service/mod.rs`) — **caught**; the hook-spawn helper without its strip — **caught** (13 + 1 tests fail); the twin's `on_task_sight_lost` call removed — **caught** by `every_loss_of_sight_path_releases_the_claims_it_strands_on_both_stores`; the twin's mark drop removed — **not caught by conformance** (L2); the A6 pin is a literal `assert_eq!`.

**G9. Low — single-write checker: a one-line `#[cfg(test)] item;` hides the next item.** `scripts/check-single-write-service.sh`. *Verified by mutation*: `#[cfg(test)] use std::fmt as _probe;` followed by an `impl` whose function makes two writes passes green. Other static residues (*verified by the reviewer*): `'{'`/`'}'` char literals counted as braces; multi-line and raw strings not blanked; `&*self.db` and `self.db.as_ref()` aliases; a helper in a second `impl` of the same type is now uncharged (G7's false positive traded for a false negative). Fix: end the skip at the attribute line's own `;` when the item is on that line; a self-test case per miss.

**G10. Low — `epic_id` guard: listed sites are checked by name only.** `tests/memory_caller_conformance/move_paths.rs::TWIN_SITES` and the module list. *Verified* (the check only tests membership). A second move added inside a listed function (`apply_set_task_epic`, `apply_create_task`, the blanket `convert.rs::from`) passes; a whole-row replacement (`*task = other;`) is missed; the "mirrors X" text is not checked. Fix: record a write count per listed site, as `single-write-service.allow` does.

**G11. Low — two looser checks.** `tests/common/script_command.rs::plugin_script_command` strips `DISPATCH_*` only (the 11-05 report suggested `env_clear()` plus an allow-list); `GIT_*` from a hook still reach the scripts' `git` calls. `scripts/check-allium.sh`'s `MIN_VERSION` is a floor, not a pin, and a stale baseline entry is only a note, so a warning that returns later is absorbed. *Verified by the reviewer.* Fix: `env_clear()` with `PATH`/`HOME`; pin the allium version (CLAUDE.md already installs `3.6.1`); fail on stale baseline entries.

## 4. Hotspots and agent context

**Hotspots** (`git log --since=60.days -- src spacetime/module/src`; change count, current lines).

| File | Changes | Lines |
|---|---|---|
| src/spacetime/module.wasm.source-hash | 142 | 1 |
| src/spacetime/module.wasm | 130 | binary |
| src/runtime/mod.rs | 79 | 1,315 |
| src/store/mod.rs | 77 | 1,669 |
| src/sync/writes.rs | 66 | 834 |
| src/spacetime/bindings/mod.rs | 66 | 3,279 (generated) |
| src/runtime/tasks.rs | 64 | 1,509 |
| spacetime/module/src/lib.rs | 64 | 358 |
| spacetime/module/src/rules.rs | 63 | 732 |
| src/service/tasks/crud.rs | 62 | 948 |
| src/sync/rows.rs | 60 | 1,660 |
| src/tui/mod.rs | 59 | 984 |
| src/sync/rules_gate.rs | 54 | 153 |
| spacetime/module/src/tasks_epics.rs | 53 | 1,552 |
| src/sync/memory_caller/mod.rs | 52 | 1,143 |
| src/sync/memory_caller/tasks_epics.rs | 47 | 1,075 |
| src/mcp/handlers/dispatch.rs | 45 | 1,083 |
| src/runtime/tests/misc.rs | 44 | 1,352 |
| src/sync/decode.rs | 42 | 689 |
| src/mcp/handlers/tests/mod.rs | 42 | 1,712 |

Changes since 2026-11-05: entered — `spacetime/module/src/tasks_epics.rs` (+243/−45 in range: create aim, loss of sight, soft delete; V1 is in it), `src/sync/memory_caller/tasks_epics.rs` (its twin), `src/sync/decode.rs`, `src/runtime/tests/misc.rs`; left — `src/tui/types.rs`, `src/runtime/tests/mod.rs`, and the removed `src/sync/tests/writes.rs` and `src/db/mod.rs`. In-range diffs of the busy-and-large five: `src/runtime/tasks.rs` (+359/−150: delete pre-checks and queues, host filter; R3, R5), `src/sync/rows.rs` (+279/−72: archive and agent rows; clean), `src/store/mod.rs` (+230/−28: archive, usage, identity reads; the fresh-area read, V3, M5), `spacetime/module/src/rules.rs` (+186/−22: aim checks, sub_status fitness, gate constants; K2's check lives here), `src/runtime/mod.rs` (+83/−47: shared quit path, signalled exit; clean). `RULES_VERSION` and `BOARD_RULES_VERSION` are both 53 and moved together on every bump in range.

**Agent context** (`wc -w`; 2026-11-05 → now): `CLAUDE.md` 2,310 → 2,307; `docs/invariants.md` 1,486 → 1,477; `docs/conventions.md` 13,096 → 13,159; `docs/reference.md` 10,051 → 10,417; `docs/testing.md` 4,883 → 6,599 (+35%); `docs/module-map.md` 4,987 → 5,011; `docs/architecture.md` 2,035 → 3,035 (+49%); `docs/how-to.md` 1,784 → 1,780; `docs/mcp.md` 2,836 → 2,857; `docs/mcp-tool-design.md` 875 → 875. All `CLAUDE.md` + `docs/*.md`: 44,343 → 47,517 (+7.2%). `CLAUDE.md` and `invariants.md` are flat; growth is in the on-demand pages (`testing.md` +100 lines in range, `architecture.md` +45), which agents read only when pointed there.

Claims checked against code: `serve_dispatch_request` is called only from `src/runtime/dispatch_host.rs`, `serve_session_request` only from `src/runtime/session_host.rs`; `unwrap_used`/`expect_used` are `warn` in `Cargo.toml`; no `state.db` write outside the service layer (MCP handlers read through it; `review_command.rs` writes through `set_setting_string`, a store call guarded in the module by `require_acting_for`); `BOARD_RULES_VERSION` equals `RULES_VERSION` (53). All hold.

## 5. Standing items

- **P5** — rustup vs a distro-pinned toolchain. Docs reworded (e9ffea03); the machine still has no rustup. A machine-setup decision for the user; raised 11-05 and now.
- **A4 residue** — the board never re-saves its repo paths, so legacy rows keep `github_path = ""` and fall back to path equality (which also widens V2). Raised 11-05 and now.

**Graduate to a check.**
- "A sibling path forgets the rule its fix added" is in its fifth run: V1 (respawn vs create), V2 (fan-out vs named row), L1 (strand refusal on detach only), R1 (reactor kills vs board kill), M2 (one bounded store command of six), K1/K2 (seed paths vs one-time migrations). Structural deliverables, one per class: (a) a module test that every public reducer inserting a `Task` from a client row runs `check_new_task_aim` and `stamp_new_task` (enumerate `#[spacetimedb::reducer]` functions that call `write_task`/`store_task` with `id: 0`); (b) one `kill_task_window` that every task-window kill uses, with a grep check banning `tmux::kill_window` on a `task-` name elsewhere; (c) `clippy::disallowed_methods` for `std::process::Command::output`/`status` outside `src/process.rs`; (d) a seed-path normaliser shared with each one-time migration, plus a restore test from a pre-migration snapshot.
- "Conformance compares generated ids across stores" (S1) is its second appearance after e6253a0a; its check is a shared creation helper that returns a per-side id map, and a grep that rejects `assert_eq!(r, m, "…same … id")`.
- The graduated guards keep missing cases (G9, G10): each fix adds a self-test case for the miss it closes.

## 6. Areas read

- Every code diff in `513f89df..d00aab72`, each assigned to one reviewer by hash; spec-only commits only where they changed code or tests (e16be068 already reviewed the specs).
- In full or near full: `spacetime/module/src/{tasks_epics,config,visibility,feed,agent_state,archive,seed,rules,lib}.rs` and their twins in `src/sync/memory_caller/`; `src/runtime/{tasks,epics,editor,commands,split}.rs`; `src/dispatch/{split_panes,worktree}.rs`; `src/startup/retire.rs`; `src/host_file/mod.rs`, `src/atomic_write.rs`; `src/keep_awake.rs`; `src/feed/{exec,cycle}.rs`; `plugin/hooks/scripts/task-status-hook`; `plugin/skills/{preference-review,retro,wrap-up}/SKILL.md`; `scripts/check-single-write-service.sh`, `scripts/check-allium.sh`; `tests/common/script_command.rs`; `tests/memory_caller_conformance/{move_paths,loss_of_sight,bookmarks,schedules,agents}.rs`.
- Fresh area: `src/store/` (`mod.rs`, `board_reads.rs`, `fixtures.rs`, `queries/{mod,board,tasks,settings,dispatch_requests,session_requests,epics,feed_requests,pr_status,learnings,usage,people,work_packages}.rs`), plus `src/spacetime/managed_store.rs` and `src/runtime/store_target.rs`; in part `src/startup/store_pin.rs`, `src/store_connection/mod.rs`; oracles `startup.allium`, `settings.allium`, `epics.allium` (excerpts).
- Not read in depth: the compact epic tree, fold and cursor commits (f5bc18d0, 3978fb40, 543acde1, 703cc17b, 226353fa), the snapshot commits (44a89d7c, 4445365c), and the store and host parts of 2f07d0b7, e0ec5e61, 9c01585b, 4cc6eeeb and most of dcbe5f97. The next run should start its fresh-area pick there or in `src/tui/`.
