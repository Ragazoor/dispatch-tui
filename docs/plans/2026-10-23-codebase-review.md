# Codebase review — 2026-10-23

Task #98463. Follows `docs/plans/2026-10-22-codebase-review-followup.md` (task #98442). This pass checks every follow-up finding against today's tree, reviews the 6 commits since that report (77c64a81..26a3b89f: cron schedules, agent-tree views and the four follow-up work packages), and runs a fresh defect hunt in the modules no recent review has read: `src/process*`, `src/repo_sync*`, `src/editor*`, `src/embeddings.rs`, `src/store_connection/`, `src/startup/`, `src/hooks/`, `src/host_file/`, `src/git*`, `src/worktree_admin.rs`, `src/main.rs`. Three read-only reviewers plus inline checks of every Medium. Paths are relative to the worktree root.

**What was run.** `cargo test --no-fail-fast` and `cargo clippy --all-targets -- -D warnings` on this host. Results are in "Test and lint run" below.

**Confidence key.** *Verified*: re-read in the code. *Confirmed*: traced through more than one file. *Plausible*: the path exists but depends on environment or timing.

## 1. Executive summary

- **The follow-up's four work packages landed and hold up.** All seven actionable items (A1, A2, T1, S1–S4, C1 and both CLAUDE.md lines) are Fixed with tests. The verdict arithmetic, the one-transaction package reducers and the `host` stamp on `RepoPath` read clean in the module and its twin.
- **The new cron scheduler has one Medium:** a satisfiable cron can stop recurring with no message, because the 8-year search window is too short for the day-of-month AND day-of-week case.
- **The new editor key can throw away unsaved edits.** `v`, Space, `a` and `q` replace or kill the editor pane with no prompt. The spec is silent; this is a decision item.
- **The fresh hunt found two Mediums in process and sync plumbing:** `run_bounded` kills only the direct child on timeout, and two repo syncs on one checkout can run at once.
- **Still open from earlier reviews:** the store's trust model (a decision), the `PaneId` newtype and the `update_task` relink ordering.

## 2. Findings

### 2.1 Architecture and patterns

The store layering keeps moving the right way: the 2026-10-22 follow-up's multi-step writes are now single reducers (`create_work_package_with_links`, `update_work_package_with_links`), and the learning retrieval check moved into `apply_learning_verdicts`. `RULES_VERSION` and `BOARD_RULES_VERSION` are both 14, each step documented. The scheduler (054df327) follows the same split: cron parsing, arming and due-ness live in `spacetime/module/src/schedule.rs`; the board only fires what the store says is due.

**A1. Medium — `run_bounded` kills only the direct child on timeout.** `src/process.rs::abandon` calls `child.kill()`; `run_bounded` never puts the child in its own process group. *Verified.* `src/feed/exec.rs` does it right (`process_group(0)` plus `kill_process_group`). Scenario: `git fetch` over ssh hangs; after 120s git is killed, but its `ssh` child keeps running and holds the stdout/stderr pipes. The `drain` reader threads block until that child dies, one leaked thread pair per hung fetch, and startup fetches once per saved repo (`exec_refresh_all_repo_sync`). The same grandchild makes the final `stderr_rx.recv_timeout(remaining())` wait out the whole deadline after git has already exited, returning empty stderr, so `repo_sync::is_push_rejection` misreads a rejected push as `Other` (*Plausible*, needs a lingering ssh ControlPersist master). Fix: spawn with `process_group(0)` and kill the group in `abandon`, as the feed runner does; after the child exits, wait for stderr only briefly.

**A2. Medium — two syncs of the same repo can run at once.** `src/tui/update/repo_sync.rs::handle_open_repo_sync_prompt` only checks `has_drift()`; nothing records a sync in flight, and the drift marker stays until the result arrives (the fetch alone can take 120s). *Verified* on the board side; the git race is *Plausible*. Scenario: the operator presses sync, `y`, then sync, `y` again. Both runs pass the clean-tree check; the second `git merge` fails on `index.lock` or `MERGE_HEAD`, and `src/repo_sync.rs::merge_origin_base` → `abort_after_conflict` runs `git merge --abort`, aborting the first run's merge. A wrap-up rebase on the same primary checkout hits the same race. Fix: keep the set of repos with a sync in flight in `App` and refuse the prompt while one runs; spec it in `docs/specs/repo-sync.allium`.

**A3. Low — a merge killed at the timeout is left half-done.** `src/repo_sync.rs::merge_origin_base`: the `GitFailure::Spawn` arm (which includes a timeout kill) returns without aborting. *Verified.* Scenario: a merge killed mid-way leaves `MERGE_HEAD`; every later sync reports the checkout as dirty until the operator cleans up by hand. Fix: call `git::abort_merge_if_in_progress` on that arm too. Fold into A2.

**A4. Low — `measure_repo` reads the default branch without expanding `~`.** `src/repo_sync.rs::measure_repo` passes `repo_path` raw to `detect_default_branch`, unlike its siblings. *Verified*, latent: today's callers pass expanded paths. Scenario: a future caller passes `~/x`; the base silently becomes `main` and the fetch fails. Fix: `expand_tilde` at the top. Fold into A2.

### 2.2 The cron scheduler (054df327)

Checked clean: OR semantics only when both day fields are restricted (a field starting with `*` counts as unrestricted, as in Vixie cron), weekday 7 folds onto Sunday, bad syntax and out-of-range values are refused, `0 0 31 2 *` is refused, UTC timestamps compare as fixed-width strings, one open request per task plus the store's own due check prevent a double fire, a restart collapses missed runs to one, and the module and twin match on arming, failed launches and leaving backlog.

**C1. Medium — a schedule the store accepted can stop recurring with no message.** `spacetime/module/src/schedule.rs::SEARCH_DAYS` is `366 * 8 + 2`; its doc says 8 years is the longest gap. That holds only for OR semantics. A day-of-month field like `*/31` (which means {1}) starts with `*`, so the day fields combine with AND, and gaps reach 11 years. *Verified.* Scenario: `0 0 */31 2 1` ("1 February when it is a Monday") matches 2027, 2038, 2044. Set in 2026 it is accepted and fires in 2027. The phoenix respawn then searches from 2027, finds nothing within 8 years, and `session_request.rs::respawn_phoenix_successor_of` returns false: no successor, no error. After a failed launch, `schedule_after_failed_launch` re-arms with `unwrap_or_default()` (`""`), whose comment says a stored schedule always answers. Set in 2028–2037 the same cron is refused as "never matches". Fix: search the 28-year Gregorian weekday cycle (about 10,300 days, still cheap), or refuse a `*`-prefixed restricted day of month together with a restricted weekday; add the AND case and the 2096→2104 leap gap as tests.

**C2. Low — the step add in `parse_field` can overflow.** `schedule.rs::parse_field`, `v += step` on `u32`. *Verified.* Scenario: `1-59/4294967295 * * * *`. In debug builds (the memory twin and tests) it panics; in the release wasm it wraps to 0, so the minute set becomes {0, 1} instead of {1}. Fix: `checked_add` and stop on `None`, or cap the step at `max - min + 1`.

**C3. Low — the twin's `create_task` leaves a repo sub-epic behind when it refuses a cron.** `src/sync/memory_caller/tasks_epics.rs::apply_create_task` routes the task (which can create a per-repo sub-epic via `find_or_create_repo_group`) before `write_task` refuses the bad cron; `validate_task_write`, whose doc promises every refusal `write_task` can make, does not check the cron. *Verified.* Scenario: `create_task` into an auto-grouping epic with `0 0 31 2 *`. The module rolls back; the twin keeps the new sub-epic. A module/twin divergence the conformance tests do not cover. Fix: run the cron check in `validate_task_write` for creates; add a conformance case.

**C4. Low (spec gap) — a phoenix task that returns to backlog by hand is never re-armed.** `schedule.rs::apply_schedule`, `tasks_epics.rs::patch_task`. *Verified*, spec-consistent. Scenario: a phoenix task fires (`next_due` is consumed), the run is abandoned, and the user moves it back to backlog. It has a schedule, no `next_due` and no `schedule_error`, so it never fires again and the card shows nothing wrong. **Decision item:** (a) arm on any write into backlog when phoenix and a schedule are set; (b) show "schedule unarmed" on the card.

**C5. Low — a close leaves stale schedule state on Done rows.** `session_request.rs::close_task` → `fail_open_dispatch_requests_of` → `task_after_failed_request`. *Verified.* Scenario 1: a schedule-origin request is open at close; the Done row gets `schedule_error = "task closed"`, hidden today only because `schedule_failed()` checks for backlog. Scenario 2: a user-origin request is open; the Done row keeps a passed `next_due`, so moving it back to backlog fires it at once, against the spec's "a one-off is spent". Fix: clear `next_due` on every close whatever the prior status, and skip the schedule mark when the task is no longer a bare claim. Fold into C4.

### 2.3 The agent-tree views (a7ebe531)

Checked clean: every tmux call targets an explicit pane id from `$TMUX_PANE`; `v` fires only on file nodes and refuses deleted files before touching tmux; no key conflicts for `s`, `v` or `BackTab`.

**G1. Medium — the editor key throws away unsaved edits.** `src/agent_tree/diff_pane.rs::show_in_editor` and `reconcile_diff_pane` (the `(true, Some(pane)) if showing_editor` and `(false, Some(pane))` arms), plus `src/agent_tree/run.rs`. *Verified.* Scenario: the user presses `v` on `a.rs`, edits it in the lower pane, goes back to the tree and presses `v` on `b.rs` or Space on a diff. Both run `respawn-pane -k`, which kills the editor with no prompt; `a`, `q` or hiding the tree run `kill-pane`. The buffer is lost. The spec's "Reuse, never takeover" guidance in `SplitAgentTreeDiffPane` refuses to kill panes because "killing the process in one of them would cost the user work", yet `OpenAgentTreeFileInEditor` says it replaces the pane with `respawn-pane -k` and never mentions unsaved work. **Decision item:** (a) while the editor shows, refuse to replace or kill the pane and say "quit the editor first"; (b) open the diff in a second pane and leave the editor alone; (c) accept and document it.

**G2. Low — the editor flag lives only in process memory.** `src/agent_tree/state.rs::RenderState::lower_pane_editor`. *Plausible.* Scenario: the tree renderer restarts without tearing down while an editor pane with the diff role is open; the flag starts `false`, so the next Space takes the "already agreeing" arm and the diff never appears. Fix: store the pane's content kind in a tmux pane option next to the role marker. Fold into G1.

**G3. Low — no tests for the editor flag's transitions.** Nothing tests how `run.rs` sets and resets `lower_pane_editor` (editor, then Space; a failed reconcile), or `v` with an empty open set followed by a toggle that leaves it empty, which kills the editor pane. *Verified.* Fold into G1.

### 2.4 The follow-up work packages (1d8c6dc6, 997d9577, e9e11066, 26a3b89f)

Checked clean: `verdict_score_change` is new minus old (helped→wrong is −2, a repeat is 0); deleting a task or learning removes its verdict rows in module and twin; both package reducers check every link before the first write; the `host` column is appended with `#[default("")]` and stamped on both arms; `tests/cli.rs` covers another host's row, an unowned row and this host's own row; the `open_set` move to `atomic_write::write_bytes`, the store-import retry removal and the `ingest_role_routed` split keep behaviour.

**W1. Low — `learning_verdicts` is not in the store snapshot.** `src/spacetime/tests/module_schema.rs` leaves it out of `SharedTable`, so `store import` and restores drop it while keeping learnings and their scores. *Verified.* Scenario: after a restore, a task that already rated a learning rates it again and the score moves twice. The `LearningVerdict` doc comment in `spacetime/module/src/tables.rs` reads as if that were the reason to leave it out. Fix: add the table to the snapshot, or reword the comment and list the gap in `docs/specs/learnings.allium`.

**W2. Low — repo paths saved before the `host` column are never pruned.** `src/sync/rows.rs::repo_paths_saved_by` skips `host == ""`; `save_repo_path` adopts a row only when it is saved again. *Verified*, and the spec states the choice. Scenario: a path deleted from disk before the upgrade stays in the repo picker forever; the board's manual delete still works. Optional fix: adopt unowned rows on the first prune, or a `--include-unowned` flag.

### 2.5 Fresh hunt: smaller findings

**F1. Low — description lines shaped like section headers eat the rest of the description.** `src/editor.rs::parse_sections` treats any `--- X ---` line as a section start. *Verified.* Scenario: a description containing `--- Notes ---` loses every line after it; one containing `--- STATUS ---` overrides the real status. The `.trim()` also strips indentation from a section's first line on every round trip. Fix: recognise only the known section names; trim only surrounding newlines.

**F2. Low — some repo-scoped learnings never match their repo.** `src/embeddings.rs::repo_ref_covers` compares against the stored `scope_ref` untrimmed; `src/mcp/handlers/learnings.rs::resolve_scope_ref` stores an explicit ref as given. *Verified.* Scenario: a learning recorded with `scope_ref = "/a/repo/"` or `"~/a/repo"` is never injected for tasks in `/a/repo`. Fix: expand `~` and trim a trailing `/` when recording.

**F3. Low — one panic disables learning injection until restart.** `src/embeddings.rs::EmbeddingService::new` runs one model thread; a panic in fastembed, or `vecs.remove(0)` on an empty result, kills it, and every later `embed` fails with "channel closed", logged only as a warning. *Plausible.* Fix: `catch_unwind` per message, or respawn the thread.

**F4. Decision item — one-shot CLI commands skip the store-identity pin.** `src/store_connection/mod.rs::open_cli_store` never calls `check_store_identity`; only the board does. *Verified.* Scenario: a stale `DISPATCH_SPACETIME_SERVER` in the shell (the cause of task #28710) makes `dispatch repo set-verify`, `plan` or `prune-repo-paths` write into the wrong store. The spec (`docs/specs/cli.allium`, `startup.allium`) is silent. Options: (a) check the pin in `open_cli_store` and refuse on mismatch; (b) document that the CLI trusts its environment.

Checked clean in the fresh areas: `backoff.rs`, `claude_paths.rs`, `plan.rs`, `worktree_admin.rs` (relative and empty `gitdir`), `host_file/` (atomic, mode before content, unknown fields kept), `startup/store_pin.rs` (matches the four paths in `startup.allium`), `startup/retire.rs` (`/proc/*/stat` split on the last `)`), `hooks/` (every request bounded), `git.rs` porcelain parsing and merge-abort guard, `main.rs` `write_snapshot`.

### 2.6 Test coverage

6,484 tests pass across 32 targets with `tmux` and `spacetime` both on `PATH`. The gaps worth closing are named with their findings: the cron AND case and century leap gap (C1), step overflow (C2), the twin create refusal in an auto-grouping epic (C3), a phoenix task returning to backlog (C4), the editor flag transitions (G3), a second sync while one runs (A2) and a hung grandchild in `run_bounded` (A1).

### 2.7 Complexity hotspots

Unchanged from the 2026-10-22 lists. The largest non-test, non-generated files are `src/tmux.rs` (1,579 lines), `src/dispatch/worktree.rs` (1,502), `src/keybindings/table.rs` (1,440, a data table), `src/runtime/mod.rs` (1,237) and `src/runtime/tasks.rs` and `src/dispatch/prompts.rs` (1,232 each). `spacetime/module/src/schedule.rs` is new at 340 lines and reads well; nothing in the new commits adds a hotspot.

### 2.8 CLAUDE.md improvements

Both follow-up items are fixed (CLAUDE.md:115 and the per-host tables line in `docs/module-map.md:17`). One new suggestion:

1. **Subprocess hygiene.** CLAUDE.md names `run_bounded` only through `docs/conventions.md`. One line saying "a child that can spawn its own children (git over ssh/https) must run in its own process group; see `src/feed/exec.rs`" would have caught A1. Better placed in `docs/conventions.md` next to `run_bounded`, once A1 is fixed.

## 3. Status of the 2026-10-22 follow-up findings

| # | Finding | Status | Evidence |
|---|---|---|---|
| A1 | `prune-repo-paths` judges every host's rows | **Fixed** | 1d8c6dc6; `src/sync/rows.rs::repo_paths_saved_by`, `tests/cli.rs`; residue W2 |
| A2 | Work-package create/update not one transaction | **Fixed** | 997d9577; `spacetime/module/src/packages.rs` |
| T1 | No prune test with a verify command | **Fixed** | 1d8c6dc6; `tests/cli.rs` |
| C1 | `ingest_role_routed` 120 lines | **Fixed** | 26a3b89f |
| S1 | Third atomic-write copy | **Fixed** | 26a3b89f; `src/agent_tree/open_set.rs` |
| S2 | Dead `store import` port retry | **Fixed (removed)** | 26a3b89f |
| S3 | Repeated `rate_learning` inflates scores | **Fixed** | e9e11066; residue W1 |
| S4 | "updated" when an unlink did nothing | **Fixed** | 997d9577 |
| §2.6-1 | CLAUDE.md unwrap sentence | **Fixed** | CLAUDE.md:115 |
| §2.6-3 | Per-host tables line | **Fixed** | `docs/module-map.md:17` |
| C2 (10-22) | `update_task` relink ordering in the service | **Open** | `src/service/tasks/crud.rs:222-236` unchanged |
| S2 (10-22) | `PaneId` newtype | **Open** | No `PaneId` in `src/` |
| — | Trust model | **Open (decision)** | No hits for "trust model" outside `docs/plans/` |

## 4. Prioritised action items

### Magic wand (top 3 by impact)

1. **Decide and write down the store's trust model** — still the one decision several findings across four reviews wait on, and F4 (CLI writes without the store pin) is one more face of it.
2. **One process-group rule for every bounded child** — `src/process.rs` (A1). Git over the network is the most common subprocess dispatch runs; a hung one leaks threads and corrupts error reporting today.
3. **`PaneId` newtype** — `src/models/tmux_window.rs`, `src/tmux.rs`, `src/dispatch/split_panes.rs`, `src/agent_tree/diff_pane.rs`. The agent tree just added another set of pane-id string arguments.

### Quick wins

- C2 — `checked_add` in `schedule.rs::parse_field`.
- A3, A4 — abort a timed-out merge; expand `~` in `measure_repo`.
- F2 — normalise `scope_ref` on record.

### Small efforts

- **WP-cron-search:** C1, C2, C3 (`spacetime/module/src/schedule.rs`, `src/sync/memory_caller/tasks_epics.rs`, tests; rebuild `module.wasm`).
- **WP-schedule-backlog:** C4, C5 (decision item; `docs/specs/schedules.allium`, `schedule.rs`, `session_request.rs`, twin; `RULES_VERSION` bump).
- **WP-editor-pane:** G1, G2, G3 (decision item; `docs/specs/agent-tree.allium`, `src/agent_tree/`).
- **WP-subprocess-and-sync:** A1, A2, A3, A4 (`src/process.rs`, `src/tui/update/repo_sync.rs`, `src/repo_sync.rs`, `docs/specs/repo-sync.allium`).
- **WP-small-fixes:** W1, F1, F2, F3 (`src/spacetime/tests/module_schema.rs`, `src/editor.rs`, `src/embeddings.rs`, `src/mcp/handlers/learnings.rs`).

### Larger efforts

- `PaneId` newtype (open since 2026-10-22).
- `update_task` relink ordering into the module (open since 2026-10-22).

### Decisions needed

- The store's trust model.
- G1: what the agent tree does with an editor pane holding unsaved work.
- C4: re-arm a phoenix schedule on a manual move to backlog, or show it as unarmed.
- F4: whether one-shot CLI commands check the store pin.
- W2 (optional): adopt unowned repo paths on prune.

## Test and lint run

`cargo test --no-fail-fast`: 32 targets, 6,484 passed, 0 failed, 1 ignored, no `skipping:` lines (`tmux` and `spacetime` both on `PATH`). `cargo clippy --all-targets -- -D warnings`: clean.
