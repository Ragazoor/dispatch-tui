# Codebase review — 2026-11-05

Task #98918. Follows `docs/plans/2026-11-04-codebase-review.md` (task #98864).

**Range.** `8687f9d4..513f89df` on `main`, 37 commits (no merges). Main themes: the 2026-11-04 work packages (feed teardown window and claim errors, refusing to aim another person's host, poll claims at task scope and served hosted tasks, the tmux bounds, `DISPATCH_TASK_ID` in hooks, `notice:` stderr lines, keep-awake fixes, `scripts/gates.sh` shared by the hook and `verify.sh`, the hardened single-write and move-path guards, detach by creator or admin, managed-feed settings in one reducer, repo paths per (path, host), bind-and-drop ports), and one feature: per-person task bookmarks (`732f22e7`, `a3ae7f13`, `b910d6d5`) with the shared mark rules (`10f17fd9`, `e036fca6`). The next run starts at `513f89df`.

**How.** Fourth run of `.claude/skills/repo-health-review`. Four read-only reviewers: (1) status and regression audit of the module and sync fix commits; (2) status and regression audit of the tmux, hooks, feed, keep-awake and gate fix commits, with mutation tests of the graduated checks; (3) a defect hunt over the bookmarks feature; (4) the fresh area, `src/runtime/`. I re-read the code for C3, A1, A3, B1, W1, G6, P1 and P3 and the rules-version history, and ran the gates.

**Gates (on `513f89df`, `./scripts/verify.sh`, the repo's verify command).**
- `scripts/gates.sh --check` (toolchain, fmt, clippy `-D warnings`, every `check-*.sh` and its self-test, including the module's own tests in `spacetime/module-tests`, which failed to link here on 2026-11-04): **green**. rustc is Fedora's 1.99.1, which now matches `rust-toolchain.toml` (513f89df moved the pin).
- `cargo test --no-fail-fast`: **red** — 13 failures, all in `src/setup/hooks/tests.rs` (C3). Fixed in this task (`fix(tests): hook script tests ignore the agent's DISPATCH_TASK_ID`); the 32 `setup::hooks` tests pass after it. Everything else: 7,839 passed, 2 ignored, no `skipping:` lines.
- Coverage not re-measured.

**Confidence key.** *Verified*: I re-read it in the code. *Verified by the reviewer*: a reviewer re-read it, I did not. *Plausible*: the path exists but the outcome depends on environment or timing.

## 1. Executive summary

- **The 2026-11-04 work packages landed: 44 of 53 items closed**, 7 partly, 3 carried "also noted" items still open. C2 is closed: the verify command is now `./scripts/verify.sh`, and clippy is green on `main`.
- **`main` failed its own verify command inside every agent window** (C3). 9ee5cd16 sets `DISPATCH_TASK_ID` on agent windows; the new `tests/hook_scripts.rs` strips it, the older `src/setup/hooks/tests.rs` does not. The fixing agent's window predated the variable, so it saw green. Fixed in this task.
- **Two Highs in old code.** Every board checks every visible task's tmux window against its own machine, and marks another machine's running agent Crashed in the shared store (W1). `create_task` lets anyone who sees a feed epic aim another person's host at a live worktree — the sibling of Q2's fix (A1).
- **Fixes that regressed or stopped short:** detach (M11's decision) reopens M10 and M12 (A2); the repo-path reducers trust a client-supplied host, now the row key (A3); bookmarks are dropped on only two of the loss-of-sight paths that release claims (B1); a failed request report sticks the row in `running` until restart (A5).
- **The graduated checks still miss cases** (G6–G8): the single-write checker stops at the first `#[cfg(test)]` in a file, which in `src/service/mod.rs` is line 14.

## 2. Status of the 2026-11-04 findings

| # | Finding | Status | Where |
|---|---|---|---|
| C1 | `main` failed clippy | Fixed | 667de454 |
| C2 | Rebased commits skip the lint gates | Fixed | 111cbd1e `scripts/gates.sh`, `scripts/verify.sh`; `get_task` shows `Verify command: ./scripts/verify.sh` |
| Q1 | Teardown rows never name the window | Fixed | 7779c3f6 reads `session_window` before `delete_task_side_effects` |
| Q2 | A viewer can aim another host's teardown | **Partly fixed** | 7779c3f6, 130a519a `rules.rs::check_task_aim` on patch/update; `create_task` unguarded (A1) |
| Q3 | Failed claim drops a request | **Partly fixed** | 7779c3f6; a failed *report* still strands the row (A5) |
| Q4 | Two boards on one host both serve | Fixed | 7779c3f6 10-minute re-claim grace |
| Q5 | `store-split.allium` pruning contradiction | Fixed | 7779c3f6 |
| Q6 | Foreign teardown logged at debug | Fixed | 7779c3f6 |
| Q7 | Twin's refresh skips sight | Fixed | 7779c3f6 |
| Q8 | Second `r` coalesces into `running` | Fixed | 7779c3f6 |
| Q9 | `ingest_feed` ignores poll ownership | Fixed | 7779c3f6 `check_ingest_claim` |
| M9 | Legacy claims stall shared feeds | Fixed | 70007f70 one-time sweep; its version constant later moved (A6) |
| M10 | Task-scoped claims outlive sight | **Partly fixed** | 70007f70 for epic-sight paths; task moves and detach do not release (A2) |
| M11 | Detach offered but always refused | Fixed (decision taken) | 92c4e19b creator, workspace admin or super-admin; the TUI still offers "— no parent —" to everyone |
| M12 | Request arrives, task does not | Fixed, regression | 70007f70 `views.rs::hosted_tasks_of` → A2 (detach) |
| M13 | Publisher claim only on module change | Fixed, regression | 299ade2e → P4 |
| M14 | Doc slips | Fixed | 299ade2e |
| M15 | Sibling decode-error branches keep rows | Fixed | 70007f70, 4dc30257 |
| N1 | PR Reviews never removes stale cards | Fixed | e2a999c1 `notice:` prefix; siblings remain (P6) |
| N2 | Delete of another host's path silent | Fixed | 8376c7e9 refuses, names the host |
| N3 | Shared path resolves on one machine | Fixed, residuals | 8376c7e9 rows per (path, host); no upgrade path (A4), host trusted (A3) |
| N4 | `setsid` children pop askpass | Fixed, regression | 299ade2e → P3 |
| N5 | Managed-feed settings partial write | Fixed | 92c4e19b `set_managed_feed_settings` |
| N7 | `build.jobs` inline table | Fixed | 299ade2e |
| N8 | First scope probe past the deadline | Fixed | 299ade2e; spec not updated (P7) |
| G3 | Move-path guard blind spots | **Partly fixed** | 67adbb28, b9501304; new misses (G8) |
| G4 | Single-write checker misses | **Partly fixed** | a856bc31; new misses (G6, G7) |
| G5 | Demanded tests partial | Fixed | 4dc30257, 293e6ea1 |
| T1 | GNOME hold end missed | Fixed | a7c40537 |
| T2 (keep-awake) | Desktop detection | **Partly fixed** | a7c40537 reads three variables, all from the tmux server's environment; `OnceLock` keeps a wrong first answer |
| T3 | No escape from `Requesting` | Fixed | a7c40537 |
| T4, T5, T6 | Epic tree | Fixed | 299ade2e |
| H1 | Hook task id from the branch | Fixed, regression | 9ee5cd16 → C3; the optional `session_id` check not done |
| H2 | Failed resume leaves a window | Fixed | 4bd2dce3; spec not updated (P7) |
| H3 | tmux bound incomplete | **Partly fixed** | 4bd2dce3; `tmux::pane_pid` still unbounded (P1) |
| H4, H5, H6, H8, H9 | tmux targeting and drift | Fixed | 4bd2dce3 |
| H7, H10 | Hook script portability; audit append | Fixed | 9ee5cd16 |
| T2 residue (standing) | `free_address` binds and drops | Fixed for the named files | 513f89df; a production sibling remains (P2) |
| 3.8 feed runner waits for stdout EOF | | Open | `src/feed/exec.rs::run_feed_command_bounded` → standing items |
| 3.8 blocking git in `sync_emission` | | Open | `src/feed/cycle.rs::sync_emission` → standing items |
| 3.8 `apply_update_epic` discards outcomes | | Open | → standing items |
| 3.8 `check-commit-msg.sh` without argument | | Moot | `gates.sh` leaves it out on purpose |

Yield: 44 of 53 closed (Fixed, Moot or decision taken); 7 partly (Q2, Q3, M10, G3, G4, keep-awake T2, H3); 3 "also noted" items open. H1, M12, M13 and N4 are closed with regressions (C3, A2, P4, P3).

## 3. Findings

Every module finding with a twin counterpart must be mirrored in `src/sync/memory_caller/` and get a case in `tests/memory_caller_conformance/`.

### 3.1 Gates

**C3. High — fixed in this task — fix for H1 → `cargo test` fails in every agent window launched after 9ee5cd16.** `src/setup/hooks/tests.rs::invoke_hook` and the `invoke` closure in `pr_learnings_hook_invokes_gate_only_for_gh_pr_create`. *Verified.* 9ee5cd16 makes both hook scripts prefer `DISPATCH_TASK_ID` and sets it on every agent window (`src/dispatch/mod.rs::agent_launch_env`). `tests/hook_scripts.rs` calls `env_remove("DISPATCH_TASK_ID")`; the older harness sets only `PATH` and inherits the rest, so 13 tests read `hook 98918 …` instead of the branch's id. Every agent's verify command is red; the agent that wrote 9ee5cd16 ran in a window launched before the variable existed. Fix applied: strip the variable in both spawns. Graduate: see §5 — a test harness that spawns a plugin script must not inherit the agent's environment; `env_clear()` plus an explicit allow-list would have caught this class.

### 3.2 Module and sync: regressions in last round's fixes

**A1. High — fix for Q2 → `create_task` can still aim another person's host at a live worktree.** `spacetime/module/src/tasks_epics.rs::create_task`, `::create_task_signalled` (`..row`, no `check_task_aim`); `feed_request.rs::teardown_request_owed`. *Verified.* Scenario: Bob sees Alice's feed epic and creates a task in it with `external_id = "x"`, `host = A1` (Alice's desktop) and `worktree = /home/alice/proj/.worktrees/999-live`. Alice's laptop A2 ingests; `x` is not in the feed, so the task is stale-deleted; the teardown is owed because the host differs from the ingesting host and Alice owns A1. A1 runs `git worktree remove --force` and `git branch -D` on the live worktree. Fix: on create, blank `host` and `worktree` (and `external_id` unless the caller acts for the feed's polling host), or run `check_task_aim` against an empty row, in both reducers; mirror in the twin; conformance case.

**A2. Medium — fix for M11 (detach) → M10 and M12 come back.** `spacetime/module/src/tasks_epics.rs::move_task_row` (no `release_task_poll_claims`); `visibility.rs::task_visible_to` (an epic-less task is its owner's only); `views.rs::hosted_tasks_of` (filtered by sight). *Verified by the reviewer.* Scenario: Bob dispatched task T and his host holds T's PR-poll claim; an admin detaches T onto the admin's board. Bob loses sight: his reactor's `get_task` answers None and the wrap-up rebase fails as "task deleted" (M12), and his task-scoped claim survives with nobody else allowed to claim it, so PR status stops (M10). A task moved into a private epic in the same workspace has the same claim gap (older). `workspaces.allium: ClaimsFollowSight` lists only `SightChanged(epic)`. Fix: release task-scoped claims after every task move; serve a task with a live agent or open request on the caller's host regardless of sight (or refuse a detach while another person's host has a live agent on it); add a detach arm to the loss-of-sight conformance test; spec first.

**A3. Medium — the repo-path reducers trust the client's `host`, which is now the row key.** `spacetime/module/src/config.rs::save_repo_path`, `::adopt_repo_path`, `::delete_repo_path`, `::set_verify_command`. *Verified.* None calls `require_acting_for`, though `save_setting` and `set_managed_feed_settings` do. Scenario: any store user calls `delete_repo_path(p, "alice-host")`; Alice's row and its verify command are gone and her feeds stop resolving `p`. Rows can be forged under her host, and any user can change any repository's verify command — which every agent of that repository runs. Fix: `require_acting_for(ctx, &host)` in all four (pass `host` to `set_verify_command`), in the twin too; conformance case.

**A4. Medium — N3 has no upgrade path for existing rows.** `spacetime/module/src/config.rs::save_repo_path`; `src/cli/commands.rs::set_verify` (skips the save when `list_repo_paths()` already has the path; its comment "saving would re-stamp it" is stale under per-host keys). *Verified by the reviewer* (the CLI path); the rest *Plausible*. On an upgraded store the second machine of a shared path still has no row, so its feeds stay unresolved until it saves through the TUI. Legacy rows keep `github_path = ""`, so commands are not shared; when a later save links two rows, both keep their old, differing `verify_command`, against `core.allium: SavedRepoPath` ("every row of one repository holds the same value"). Fix: `set_verify` saves when `list_repo_paths_saved_by_this_host` lacks the path; the board re-saves its own paths once at startup to stamp `github_path`; a save that links siblings unifies the command.

**A5. Medium — fix for Q3 + Q4 → a failed report leaves a request `running` until the board restarts.** `src/feed/requests.rs::serve_teardowns`, `::rows_to_serve`; `src/feed/mod.rs::spawn_refresh_request`; `feed_request.rs::validate_feed_request_claim`. *Verified by the reviewer.* After a failed done/failed report, `forget` drops the id; the next tick re-claims; the module refuses inside the 10-minute grace; the board reads `Ok(false)`, puts the id back in `served`, and `retain` keeps it while the row is open — so this process never retries, even after the grace. Open rows are not pruned. Fix: retry the report, not the claim (a pending-report set), or `forget` on a grace refusal.

**A6. Low — `FEED_SETTER_CLAIMS_RULES_VERSION` moved 37 → 38 in 8376c7e9.** `spacetime/module/src/rules.rs`. *Verified.* 70007f70 added the sweep at 37; 8376c7e9 bumped both constants together, apparently a replace-all. Stores already on 37 re-run the sweep (idempotent today), and the constant's doc ("the version that added the sweep") is false. The next mechanical bump may move it again. Fix: restore 37; a test pins it.

**A7. Low (plausible) — verify-command reads and writes pick a row by path alone.** `config.rs::set_verify_command` (`find(|r| r.path == path)`), `src/sync/rows.rs::verify_command` (first row by path, any host). With rows per (path, host), two machines holding different repositories at the same path read or set each other's command; module iteration order differs from the twin's BTreeMap, so the two can diverge. Fix: select this host's row; take the sibling set from it.

**A8. Low (plausible) — a detach leaves package links and task learnings in the old workspace.** `tasks_epics.rs::move_task_row` (the workspace-change arm does not treat epic → none as leaving). Against `LinkedItemsShareThePackagesWorkspace`; reachable by creators and admins since 92c4e19b. Fix: treat a detach as leaving the workspace.

### 3.3 Bookmarks (new feature)

Authority and sight hold: `set_bookmark` checks own identity and `visible_task` before writing; the table is private; `own_bookmarks` returns only the caller's rows on visible tasks. Rules versions moved together. Every Done path goes through `store_task`, which ends marks.

**B1. Medium — bookmarks and unseen marks survive most loss-of-sight paths.** `spacetime/module/src/bookmark.rs::drop_marks_where` is called only from `workspace.rs::drop_follows_in_workspace` and `visibility.rs::move_epic_row`; `config.rs::release_claims_without_sight` is called from three more places (`visibility.rs` `set_epic_visibility`, `tasks_epics.rs` epic re-parent, `feed.rs` managed root turned private) that drop no marks; task moves drop neither. *Verified.* Scenario: Bob bookmarks T in epic E; Alice makes E private. Bob's row stays as a hidden orphan he cannot remove (`set_bookmark(false)` answers "task does not exist"); when E is public again the bookmark reappears. `core.allium: Bookmark` says a row lives until "its person loses sight" and also that it "survives the task moving to another epic" — a move into an epic the person cannot see satisfies both. Fix: settle the spec (end, or hide and return), then one `on_sight_lost` helper that releases claims *and* drops marks, called from every path; extend `every_loss_of_sight_path_releases_the_claims_it_strands`. This is the "sibling path" class again (§5).

**B2. Medium — `update_tasks` with fields and `bookmarked` is two reducer calls.** `src/mcp/handlers/tasks/crud.rs::handle_update_task`. *Verified by the reviewer.* `update_task` then `set_bookmark`; if the second is refused (identity not settled, or another host closed the task between them), the title is written, the reply is an error and `notify_task_changed` is skipped. `mcp-task-tools.allium: UpdateTaskViaMcp` has both effects in one rule. Fix: carry the bookmark in the same reducer, or check its preconditions first and report a partial success honestly.

**B3. Low — a lone `url_type` with `bookmarked` is silently dropped.** `handle_update_task` (`if !fields.is_empty() || bookmarked.is_none()`). `{url_type: "pr", bookmarked: true}` neither applies nor refuses `url_type`. *Verified by the reviewer.* Fix: refuse `url_type` without `url` before branching.

**B4. Low — conformance gaps and a dead "done write".** `tests/memory_caller_conformance/bookmarks.rs` covers set, clear, a missing task and a merged PR; not `move_epic_to_workspace`, view parity after loss of sight, `batch_delete`/`delete_epic`, or member removal on the twin. `src/sync/tests/queries.rs` does not list `own_bookmarks`. 9450c870's `end_marks_on_finish` in `feed_ingest.rs::ReducerStore::update_task` sits on a path that never sets Done (the routing move). *Verified by the reviewer.* Fix: add the cases; reword the comment.

**B5. Low — `drop_marks_where` scans whole tables; the operator may write anyone's bookmark.** `bookmark.rs::drop_marks_where` lost the `subscriber` index on member removal (10f17fd9). `require_own_identity` and `visible_task` pass for the operator, while `SetBookmark` says `subscriber = caller()`. *Verified by the reviewer* / *Plausible*. Fix: take an optional subscriber; state or forbid the operator exception.

### 3.4 Fresh area: `src/runtime/`

**W1. High — every board checks every visible task's window against its own machine, and crashes other machines' agents.** `src/tui/update/agent.rs::tick_window_checks` (no host filter), `src/runtime/tasks.rs::exec_batch_check_windows` (`list-windows -a` on this machine), `::handle_window_gone` → `::handle_agent_crashed`. *Verified* (board side). `visible_agents` serves agents of every held task, so a teammate's running task — or your own on a second machine — shows a `tmux_window` on this board. Board B never finds it, sends `WindowGone`, writes sub_status Crashed with `releasing_tmux_window()`, clears subagents and notifies "agent crashed". The module does not refuse it (`check_task_aim` judges only naming a window). Host A's `tick_sub_status` skips Crashed tasks, so the card stays Crashed while the agent runs, and "press d to retry" can start a second agent. `tick_pr_poll` has the filter this lacks (`t.host.is_none() || t.host == local_host_id`) and a test for it. Fix: the same filter in `tick_window_checks` and `tick_sub_status` (which also reclassifies foreign Running tasks with this machine's clock); a test with a foreign-hosted Running task; state it in `agent-health.allium: DetectCrashedAgent`.

**W2. Medium — one failed tmux query reads as "split pane closed", then crashes the pinned agent.** `src/runtime/split.rs::exec_check_split_pane` → `tmux::pane_exists` (`unwrap_or(false)`); `exec_respawn_split_pane` sends `PaneClosed` on any error. *Verified by the reviewer.* A timeout resets the split (`SplitPaneClosedResets`) while the pane is still joined; next tick `tick_window_checks` no longer skips the task, its standalone window does not exist, and W1's path marks it Crashed. Contradicts `has_window_or_assume_present`. Fix: `pane_exists` returns `Result`; do nothing on `Err`.

**W3. Medium — saving the task editor writes a stale snapshot over changes made while it was open.** `src/runtime/editor.rs::finalize_task_edit`, `apply_task_editor_fields`. *Verified by the reviewer.* The snapshot is taken at open and `UpdateTaskParams` always sends status, title, model, base branch, tag, wrap-up mode and phoenix. An agent wraps up to Review while you fix a description; saving puts it back to Running. `finalize_epic_edit` has the same shape. Fix: send only the fields the user changed against the snapshot.

**W4. Medium — a refused task edit is still shown and its repo path still saved.** `editor.rs::finalize_task_edit` applies `TaskMessage::Edited` and `exec_save_repo_path` after an `Err` from `update_task` (e.g. `check_task_aim`). `finalize_epic_edit` returns early and says why. *Verified by the reviewer.* Fix: return early.

**W5. Medium (plausible) — quitting can hang on blocking threads.** `src/runtime/mod.rs::spawn_input_thread` leaves only when `key_tx.send` fails on a key event; `src/main.rs` drops the runtime with no `shutdown_timeout`, which waits for every `spawn_blocking` task. After quit the process waits for the next keypress, or for an open pop-out editor to close. Fix: check `key_tx.is_closed()` each poll; `runtime.shutdown_timeout(..)`.

**W6. Medium — the tick refresh is not the backstop the docs say.** `src/runtime/tasks.rs::exec_refresh_from_db` reads tasks, epics, unseen, bookmarks and dispatch requests; `reload_board` also reads work packages and links, repo paths and base branches. `docs/architecture.md` says "Both paths read the same set of tables". *Verified by the reviewer.* Fix: one table list for both, or correct the doc.

**W7. Low — two boards in one tmux server step on each other.** `mod.rs::teardown_tmux_for_tui` unbinds Prefix+Space and Prefix+e server-wide; `editor.rs::exec_pop_out_editor` restores focus with `select_window(TUI_WINDOW_NAME)`, a server-wide name lookup (H6's class). *Verified by the reviewer.* Fix: target `self_pane`; unbind only when no other board is running.

**W8. Low (plausible) — a signal skips the tmux teardown; a relaunch can kill a pinned agent.** `store_target.rs::clean_up_on_termination` calls `process::exit(0)` on SIGHUP/SIGTERM, skipping split restore and unbinds. `src/startup/retire.rs::retire_board_window_with` kills the old board window without breaking a pinned agent pane out. Fix: run the split restore before exit and before the retire kill; state it in `split-pane.allium`.

**W9. Low — partial writes and dropped errors in runtime helpers.** `src/runtime/epics.rs::exec_toggle_epic_group_by_repo` writes `update_epic(group_by_repo)` then `regroup_epic`/`flatten_epic`; a failed second call leaves the flag set and tasks unmoved. `tasks.rs::exec_quick_dispatch` ignores `save_repo_path`'s result and leaves a Backlog task when the dispatch request fails after `create_task`. *Verified by the reviewer.* Fix: one reducer for the toggle (the flag and the regroup); report or roll back in quick dispatch.

**W10. Low — blocking work on the command-drain path.** `tasks.rs::grant_trust`, `::check_trusted` await `spawn_blocking` on `~/.claude.json` (several MB); `src/tui/update/system.rs::handle_repo_paths_updated` calls `Path::is_dir()` per path inline, on every pushed row. A stalled mount freezes the board. *Verified by the reviewer.* Fix: report through `msg_tx` instead of awaiting; check directories off the update loop.

Also read, not filed: `SessionRequestObserver::open_exits` (`src/dispatch/request_host.rs`) only grows; `session_host.rs::recover_session_requests` may read before the sweep's result arrives (the pump corrects it).

### 3.5 Process, tmux and feeds: residues of last round's fixes

**P1. Low-Medium — fix for H3 → `tmux::pane_pid` is still unbounded.** `src/tmux.rs::pane_pid` uses `runner.run`; called from `src/startup/retire.rs` on relaunch. `run_checked_timeout`'s doc now says every tmux call goes through it. *Verified.* Fix: bound it.

**P2. Medium (plausible) — T2 sibling: `store_import` picks a port by bind-and-drop.** `src/cli/store_import.rs::spawn_on_free_port`; `wait_until_answering` accepts any store answering on the port, so a lost race dumps another store's data instead of the copy. Test-only siblings: `managed_store.rs::a_port_nothing_listens_on_probes_as_nothing_listening`, `store_pin_tests.rs::nothing_listening_reads_as_none`. The code path is verified by the reviewer; the race outcome is plausible. Fix: check the listener's data directory, as `managed_store_real.rs` now does.

**P3. Low — fix for N4 → the user's `SSH_ASKPASS_REQUIRE` is overridden.** `src/process.rs::non_interactive_env` sets `never` always; the same function keeps an existing `GIT_SSH_COMMAND` for exactly this reason. A user with `force` and a keyring askpass can no longer unlock keys in bounded children. *Verified.* Fix: set it only when unset.

**P4. Low (plausible) — fix for M13 → an unbounded `spacetime call` on every warm start.** `SpacetimeCliPublisher::claim_store_owner` uses `self.runner.run`; a store that accepts connections but stalls on reducers hangs startup. Fix: `run_with_timeout`.

**P5. Low — the toolchain pin follows the distro.** 513f89df moved `rust-toolchain.toml` to Fedora's 1.99.1; `verify.sh` runs `check-toolchain.sh`, so the next `dnf` rustc update turns every agent's verify red until someone bumps the pin. CLAUDE.md and `verify.sh` say verify "fails on a distro toolchain"; it fails only on a version mismatch. *Verified by the reviewer.* Fix: install rustup on this machine (CLAUDE.md's first-time setup), or reword the docs. Decision for the user.

**P6. Low — N1's siblings: other permanent config warnings still taint every feed cycle.** `scripts/fetch-reviews.sh` and `scripts/fetch-dependabot.sh` print the bots.conf format and duplicate-entry warnings without `notice:`. A misconfigured file makes every cycle additive. *Verified by the reviewer.* Fix: prefix them; a test per script under a bad config.

**P7. Low — rules the code now enforces that no spec states.** H2 (resume kills its half-created window: `dispatch.allium: ResumeFailed`), N8 (scope probe skipped under 30 s left, sccache bounded by time left: `dispatch.allium`), H5/H6 (own-pane targeting, rename skipped without `$TMUX_PANE`), `non_interactive_env`'s variables; `docs/reference.md` "Verify Command" still says per-`repo_path` row. Also `contains_focus_events` misses `set -s focus-events on` and a trailing comment (appends a duplicate line); a0e568fc's hint test skips the hardcoded mode bars in `status_bar.rs`. *Verified by the reviewer.* Fix: `allium:tend` for the specs; small code fixes.

**P8. Low — keep-awake desktop detection still reads only the tmux server's environment** (T2 residue). `src/keep_awake.rs::hold_command_for_this_machine`. Fix: try `gnome-session-inhibit` first when it is on `PATH`, or read `tmux show-environment -g`; drop the `OnceLock` caching of a fallback answer.

### 3.6 The graduated checks (audited by mutation)

**G6. Medium — `check-single-write-service.sh` stops reading a file at its first `#[cfg(test)]`.** The awk rule `/^#\[cfg\(test\)\]/ { flush(); fn = ""; exit }`. *Verified.* `src/service/mod.rs` has `#[cfg(test)] pub use …` at line 14, so nothing below it is checked; `src/service/api.rs` from line 641. Fix: skip only the `mod tests { … }` block; a self-test case with a mid-file attribute.

**G7. Low — single-write checker misses not in its header.** *Verified by the reviewer* (probe files). `let store = Arc::clone(&self.db);` rebinding; `//` inside a string literal strips the rest of the line; same-named helpers in two `impl` blocks share a count (false positive). Already admitted: helpers in another file, two levels deep, `join_all(...map(...))`. Fix: handle the first three; list the rest in the header.

**G8. Low — move-path `epic_id` guard misses.** `tests/memory_caller_conformance/move_paths.rs::epic_id_writers`. *Verified by the reviewer* (scanner compiled standalone). Caught now: one-line literals, `pub(super) async fn`, subdirectories. Missed: values starting with `Option`, `pub`, `i64` or `Patch` (`epic_id: Option::from(e)`, `epic_id: public_epic`) are filtered as declarations; `Self { epic_id, ..self }`; `.epic_id.replace(`, `&mut t.epic_id`; a `"}"` in a string ends the span; `#[reducer] pub fn` on one line charges the previous function; twin-only move functions (`src/sync/memory_caller/` is not scanned). Fix: judge by what follows `epic_id:` only; scan `Self {` in `impl Task`; treat `&mut …epic_id` and `.replace(` as writes; scan the twin.

## 4. Hotspots and agent context

**Hotspots** (`git log --since=60.days -- src spacetime/module/src`; change count, current lines).

| File | Changes | Lines |
|---|---|---|
| src/spacetime/module.wasm.source-hash | 107 | 1 |
| src/spacetime/module.wasm | 99 | binary |
| src/runtime/mod.rs | 71 | 1,279 |
| src/spacetime/bindings/mod.rs | 59 | 3,214 (generated) |
| spacetime/module/src/lib.rs | 59 | 341 |
| src/sync/writes.rs | 57 | 781 |
| src/store/mod.rs | 57 | 1,467 |
| src/service/tasks/crud.rs | 56 | 924 |
| src/tui/mod.rs | 52 | 940 |
| src/runtime/tasks.rs | 50 | 1,300 |
| spacetime/module/src/rules.rs | 48 | 568 |
| src/sync/rows.rs | 47 | 1,453 |
| src/sync/memory_caller/mod.rs | 43 | 1,062 |
| src/tui/types.rs | 42 | 585 |
| src/mcp/handlers/dispatch.rs | 42 | 1,062 |
| src/runtime/tests/mod.rs | 41 | 984 |
| src/sync/tests/writes.rs | 40 | removed |
| src/sync/rules_gate.rs | 40 | 153 |
| src/mcp/handlers/tests/mod.rs | 40 | 1,699 |
| src/db/mod.rs | 40 | removed |

Changes since 2026-11-04: `src/sync/rules_gate.rs` entered (four constant bumps); `src/models/tasks.rs` left. In-range diffs of the busy-and-large five: `rules.rs` (+77) adds `check_task_aim` and four version notes (A1 is its missing create arm; A6 the constant). `src/sync/rows.rs` (+101) is M15 and the bookmark rows; clean. `src/runtime/tasks.rs` (+30) and `src/runtime/mod.rs` (+23) are bookmarks wiring and H6; the fresh-area read covered both (W1, W6). `src/store/mod.rs` (+25) adds bookmark reads; clean.

**Agent context** (`wc -w`; 2026-11-04 → now): `CLAUDE.md` 2,281 → 2,310; `docs/invariants.md` 1,486 → 1,486; `docs/conventions.md` 13,096; `docs/reference.md` 10,051; `docs/testing.md` 4,648 → 4,883; `docs/module-map.md` 4,987; `docs/architecture.md` 2,035; `docs/how-to.md` 1,784; `docs/mcp.md` 2,836; `docs/mcp-tool-design.md` 875. All `CLAUDE.md` + `docs/*.md`: 44,079 → 44,343 (+0.6%). Flat.

Claims checked against code: `serve_dispatch_request` is called only from `src/runtime/dispatch_host.rs`, `serve_session_request` only from `src/runtime/session_host.rs`; `BOARD_RULES_VERSION` equals the module's `RULES_VERSION` (39); `unwrap_used`/`expect_used` are `warn` in `Cargo.toml`; the PR seam holds (`src/runtime/pr.rs` reads the raw state only to choose host effects, the module derives status). All hold. One doc claim is false: `docs/architecture.md` "Both paths read the same set of tables" (W6). C2 removed CLAUDE.md's prose rule about running the check scripts; the verify command now enforces it.

## 5. Standing items

- **Feed runner waits for stdout EOF** — `src/feed/exec.rs::run_feed_command_bounded`. Raised 11-03, 11-04, now. Same class as T1, fixed only in keep-awake.
- **Blocking git in `sync_emission`** — `src/feed/cycle.rs::sync_emission` calls `git remote get-url` (10 s each) inside an async function. Raised 11-04, now.
- **`apply_update_epic` discards outcomes** — `src/sync/memory_caller/tasks_epics.rs`. Raised 11-04, now. Safe while the dry run matches.
- **P5** — rustup vs a distro-pinned toolchain. A machine-setup decision for the user.

Filed: all findings are in epic #69690 (tasks #98990–#98999, plans in `docs/plans/review-2026-11-05/`). The user chose its own package for the three carried feed-runner items (task #98998). Decision items: A2 and B1 (#98992), A4 (#98993), B5 (#98995), W7 (#98990), P5 (#98999).

**Graduate to a check.**
- "A sibling path forgets the rule its fix added" appeared a fourth run running: A1 (create vs patch), A2 (task move vs epic sight), B1 (marks vs claims), C3 (old hook test harness vs new), P6 (other feed scripts). The deliverable is structural: one `on_sight_lost(ctx, tasks)` in the module that releases claims and drops marks, so a new loss-of-sight path cannot do half; and one shared helper for spawning plugin scripts in tests that strips the agent's `DISPATCH_*` variables (or starts from `env_clear()` with an explicit allow-list, keeping `PATH` and `HOME`).
- "A test passes in one agent's environment and fails in another's" (C3) is new; the harness helper above is its check.
- The graduated guards keep missing cases (G6–G8); each fix adds a self-test case for the miss it closes.

## 6. Areas read

- Every diff in `8687f9d4..513f89df`.
- In full or near full: `spacetime/module/src/{feed_request,rules,config,views,tasks_epics,bookmark,visibility}.rs` and their twins in `src/sync/memory_caller/`; `src/feed/{requests,mod}.rs`; `src/setup/hooks/tests.rs`, `tests/hook_scripts.rs`, both plugin hook scripts; `scripts/gates.sh`, `scripts/verify.sh`, `scripts/check-single-write-service.sh`; `tests/memory_caller_conformance/{move_paths,bookmarks}.rs`; `src/mcp/handlers/tasks/crud.rs` (update paths); `src/keep_awake.rs`; `src/process.rs` (`non_interactive_env`).
- Fresh area: `src/runtime/` (`mod.rs`, `event_loop.rs`, `tasks.rs`, `commands.rs`, `dispatch_host.rs`, `session_host.rs`, `split.rs`, `editor.rs`, `epics.rs`, `pr.rs`, `store_target.rs`, `people.rs`, `work_packages.rs`, `learnings.rs`, `settings.rs`, `keep_awake.rs`, `budget.rs`) and `src/dispatch/request_host.rs`; in part `src/tui/update/agent.rs` (ticks, window-gone, crash), `src/startup/retire.rs`, `src/main.rs`; oracles `agent-health.allium` (`DetectCrashedAgent`), `split-pane.allium` (`SplitPaneClosedResets`).
