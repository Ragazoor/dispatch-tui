# Codebase review — 2026-10-28

Task #98508. Follows `docs/plans/2026-10-23-codebase-review.md` (task #98463). This pass checks every 2026-10-23 finding against today's tree, reviews the 59 commits since that report (69c424fc..8e1a4654: the five follow-up work packages, workspaces and display names, `resume_task`, the per-task model picker, the My Reviews signal, the startup owner check and smoke tests, and the agent-tree rework), and asks one architecture question: which data rules still live in `src/service/` rather than the module. Three read-only reviewers plus inline checks of every Medium. Paths are relative to the worktree root.

**What was run.** `cargo test --no-fail-fast` and `cargo clippy --all-targets -- -D warnings` on this host. Results are in "Test and lint run" below. Coverage was not re-measured; the last figure is 92.17% (2026-10-15, `docs/testing.md`).

**Confidence key.** *Verified*: re-read in the code. *Plausible*: the path exists but depends on environment or timing.

## 1. Executive summary

- **The 2026-10-23 work packages landed well.** 15 of 18 findings are Fixed with tests; A2 and F1 are partly fixed, W2 stays optional. The process-group kill, the 40-year cron search and the editor-pane content marker all read clean.
- **Three new Mediums in the new features:** a feed move between review buckets keeps the unseen mark (spec breach), the display-name check misses most of Unicode's format category, and `resume_task` answers success for a task the board will refuse.
- **The agent tree's `v` key still runs the pane-close path** the spec says it must not; only a swallowed `set-option` error stands between it and killing the editor just opened.
- **The fresh-install smoke test leaves the first board's background tasks running** into the second launch on the same address.
- **Still open:** the store's trust model (a decision), `PaneId` newtype, and `update_task`'s two-write relink, now joined by a second service-side rule (the Done transition read from a cached snapshot).

## 2. Findings

### 2.1 Architecture and patterns

The layering keeps moving the right way: schedules, verdicts, package links and display-name validation all live in `spacetime/module/src/` with twin mirrors, and `RULES_VERSION` = `BOARD_RULES_VERSION` = 15, pinned equal by a test. Two service-side rules remain, both in `src/service/tasks/crud.rs::TaskService::update_task`.

**S1. Medium — relink and patch are two writes.** `src/service/tasks/crud.rs:222-242` (`TaskService::update_task`). Open since 2026-10-22, now ordered by whether the repo changes, but still two store calls. *Verified.* Scenario: an MCP `update_tasks` call sets `repo_path`, `epic_id` and a `sub_status` that is illegal for the status. The relink runs, then the module refuses the patch. The caller gets an error, but the task has already moved epic and kept its old repo. Fix: one module reducer that takes the epic in the patch; mirror it in `src/sync/memory_caller/tasks_epics.rs` and add a conformance case.

**S2. Medium — the Done transition is decided from the subscription cache.** `src/service/tasks/crud.rs:197-220, 253` (`TaskService::update_task`); same pattern in `src/service/epics.rs:380-384` (`EpicService::update_epic`). *Verified* pattern, failure *Plausible.* The service reads the prior status from the board's cached rows, decides `entering_done` from it, and clears `patch.completed_at` because the module "lets a completion the patch names win". The same snapshot decides `ClosePrOnDone`. Scenario: another board reopens the task; this board's cache still says Done; a write from this board then carries a stale `completed_at`, and the Done column orders the card wrong. Fix: let the module own `completed_at` on every transition into Done (ignore a client value unless an explicit reorder is flagged) and report the transition back. Fold into S1.

**S3. Low — the service re-checks existence before writing.** `crud.rs::create_task_inner`, `move_task_to_epic`, `update_task` call `require_epic_accepting_work` / `task_exists` first. *Verified.* Scenario: the epic is deleted between check and write; the module's refusal surfaces as `Internal`, and a deleted task can yield a silent `Ok`. Also, the doc comment on `src/service/epics.rs::delete_epic` says the store "does not check" the deletable guard, but `spacetime/module/src/tasks_epics.rs:706` now does. Fix: map the module's refusal to the same error the pre-check gives, and correct the comment. Fold into S1.

### 2.2 New features: workspaces, resume, feeds

Checked clean: `set_display_name` shares `validate_display_name` between module and twin and has a conformance case; profile registration runs on every connect; `shown_name` matches on both sides; the model picker stores and carries `model` through phoenix successors and resumes; the startup owner check matches `AbortWhenTheCliDoesNotOwnTheManagedDatabase`; `leave_hidden_views` (c7d47a91) terminates and guards cycles.

**N1. Medium — a feed move between review buckets keeps the unseen mark.** `spacetime/module/src/feed_ingest.rs:445-452` (`route_and_move`) calls `ReducerStore::update_task` (`feed_ingest.rs:111-113`), which only writes the row; the twin `src/sync/memory_caller/feed.rs:440-443` does the same. *Verified.* The spec rule `UnseenMarkEndsWithTheTaskLeavingItsEpicOrFinishing` (`docs/specs/tasks.allium`) says any epic change ends every mark, and `set_task_epic` honours it via `clear_unseen_for_task`. Scenario: a PR lands in My Reviews and is marked; the direct request is dropped while a team request remains, so the next poll moves it to Team Reviews with the mark still on. The repo-group moves in `ingest_grouped` have the same gap. Fix: clear the task's marks in `update_task` when `epic_id` changes, module and twin; add "a move out of My Reviews clears the mark" to `tests/memory_caller_conformance/feeds.rs`.

**N2. Medium — the display-name check misses most format characters.** `spacetime/module/src/user_profile.rs:25-37` (`is_format_char`, used by `validate_display_name`). *Verified.* The spec (`docs/specs/workspaces.allium`, `SetDisplayName`) refuses all control and format characters; the code hand-lists a subset. Tag characters U+E0001 and U+E0020–E007F pass (the standard way to hide text an LLM reads, which is the threat the comment names), as do U+FFF9–FFFB, U+0600–0605, U+06DD, U+070F, U+08E2 and U+1D173–1D17A. U+2028/U+2029 render as line breaks and also pass. The twin calls the same function, so the conformance case (`"Al\nice"` only) cannot catch it. Fix: a complete Cf range table (or the `unicode-general-category` crate, if it builds for wasm), plus U+2028/U+2029; add a tag-character case to `tests/memory_caller_conformance/scenario.rs`. Module change: rebuild `module.wasm`.

**N3. Medium — `resume_task` answers success for a task the board will refuse.** `src/mcp/handlers/tasks/resume.rs::refusal` (lines 70-96) and `::handle_resume_task` (45-64); `src/tui/update/agent.rs::handle_resume_task` (646-689). *Verified.* The spec (`docs/specs/mcp-task-tools.allium`, `ResumeTaskViaMcp`) requires `task.is_locally_owned` and lists "its worktree is on another machine" as a tool error. The handler deliberately leaves the host gate to the board (its comment says so), answers "resume requested … the board is restarting its agent", and the board then refuses on the status line only. It also returns `vec![]` with no message when the task left the board's view or changed between the read and the event. Scenario: an agent resumes a teammate's task and waits for a session that never starts. Fix: plumb `HostStore::host_id` (`src/store/queries/settings.rs`) into `McpState` and refuse in `refusal`; give the board's silent branches a status message. **Decision item** only if the user prefers to change the spec to match the code.

### 2.3 Agent tree (61a05637 … 18c7d51f)

Checked clean: the one-open-file rule (`state.rs::toggle_diff`, deduplication in `read_open_set`), the fallback when a selected commit disappears, `ListCursor` clamping on empty lists, explicit pane targets on every tmux call, the hex check before a commit id reaches git.

**G1. Medium — `v` runs the close path the spec says it must not.** `src/agent_tree/run.rs:140-143` (`open_in_editor` → `publish_open_set` → `diff_pane.rs::reconcile_diff_pane(false)`). *Verified* divergence, failure *Plausible.* `OpenAgentTreeFileInEditor` says emptying the set this way is not an `AgentTreeDiffOpenSetChanged` and does not close the pane. The code reconciles anyway; it is harmless only while `diff_pane.rs::mark_content(EDITOR)` succeeds, and that function logs and swallows its error (`diff_pane.rs:226`). Scenario: a diff is open, the user presses `v`, the `set-option` fails; the pane still reads as "diff", reconcile takes the `(false, Some((pane, false)))` arm and kills the editor just opened. Fix: in `open_in_editor`, write the emptied set with `open_set::write_open_set` and skip the reconcile.

**G2. Low — the lower pane's role and content are read in two tmux calls.** `src/agent_tree/diff_pane.rs:55-72` (`existing_lower_pane`). *Plausible.* Scenario: the user quits the editor between the calls; the pane counts as a diff and `kill_pane` hits a pane that is gone, showing a spurious notice. `src/dispatch/agents.rs::toggle_agent_tree_pane` avoids this (comment near line 246). Fix: one `list-panes` with both options in the format. Fold into G1.

**G3. Low — a second `$TMUX_PANE` parser.** `src/agent_tree/diff_pane.rs:42` (`current_pane_from_env`) checks only non-empty; `tmux::self_pane_id` checks `is_pane_id`. *Verified.* Fix: use `tmux::self_pane_id`. Fold into G1.

**G4. Low — an error exit orphans the diff pane.** `src/agent_tree/run.rs:293-303` (`run_loop`): a `?` from `terminal.draw` or `event::poll` skips `tear_down_diff_pane`, breaking `KillAgentTreeDiffPaneWithItsTree`. `src/agent_tree/pane.rs:378-380` (`with_pane_terminal`) leaves raw mode on if `EnterAlternateScreen` fails. *Verified.* Fix: a drop guard for teardown and terminal restore. Fold into G1.

**G5. Low — key presses starve the refresh.** `src/agent_tree/run.rs:303-333` and `src/agent_tree/diff_viewer.rs:731-750` refresh only when the 1-second `event::poll` times out. *Verified.* Scenario: holding `j` or scrolling a diff stops git refreshes for as long as keys arrive; `RefreshAgentTree` is defined on elapsed time. Fix: track the last refresh and refresh when due, key or no key. Fold into G1.

**G6. Low — stale doc comments after 18c7d51f.** `src/agent_tree/open_set.rs::write_open_set` ("the order is part of the message") and `src/agent_tree/diff_viewer.rs::LastSeen` ("ORDER INCLUDED"); the set is now a `BTreeSet` of at most one path. *Verified.* Fold into G1.

### 2.4 Fix commits for the 2026-10-23 findings

**P1. Low — a remote that prompts now hangs until the timeout.** `src/process.rs:269` (`run_bounded`, `process_group(0)`). *Plausible.* A child in a background process group that reads `/dev/tty` (ssh passphrase, HTTPS credential prompt) is stopped by SIGTTIN. Scenario: `dispatch repo sync` in a terminal with a passphrase key and no agent used to prompt; now it reports "timed out after 120s", hiding the cause. Fix: set `GIT_TERMINAL_PROMPT=0` and `GIT_SSH_COMMAND='ssh -o BatchMode=yes'` for bounded git calls so they fail at once with git's own message; note it next to `run_bounded` in `docs/conventions.md`.

**P2. Low — a wrap-up fast-forward can race a repo sync.** `src/dispatch/finish.rs:239` (`fast_forward_base`, `git merge --ff-only` in the primary checkout) does not consult `App::repo_syncs_in_flight` (`src/tui/mod.rs`). *Verified* gap, race *Plausible.* Residue of A2 (10-23). Scenario: a sync's merge and a wrap-up's fast-forward run on one checkout; one fails on `index.lock` and the sync's abort path may abort the other's merge. Fix: refuse a sync while a wrap-up rebase of that repo runs, and the reverse; extend `OneSyncInFlightPerRepo` in `docs/specs/repo-sync.allium`.

**P3. Low — single-line editor fields keep leading spaces.** `src/editor.rs::section_value` (line 172) no longer trims the start, which F1's fix needed for DESCRIPTION, but it feeds TITLE, REPO_PATH, BASE_BRANCH, PLAN, URL and FEED_COMMAND too (`parse_editor_output` around lines 510-519). *Verified.* Scenario: ` main` typed in BASE_BRANCH is stored with the space and later git calls fail. Fix: trim the single-line sections where they are read.

**P4. Low — a description line equal to a known header still ends the description.** `src/editor.rs::parse_sections_in`. *Verified.* Residue of F1 (10-23). Scenario: a description containing a line `--- STATUS ---` loses the lines after it and can override the real status. Fix: only treat a header as a section start when it is the next expected section in order, or escape such lines on write. Fold into P3.

**P5. Low — the CLI's store-pin refusal uses the board's wording.** `src/store_connection/mod.rs::check_store_identity` returns `StartupAbort::StoreSwitched`, whose text (`src/startup/launch.rs:325-336`) says "the board has not started … run `dispatch tui` again". *Verified.* Fix: a CLI-specific message ("this command wrote nothing; unset `DISPATCH_SPACETIME_SERVER`, or run `dispatch tui --accept-store-switch`").

### 2.5 Startup smoke tests

Checked clean: no fixed ports, sleeps are deadline-bounded and annotated, waits use the change signal, `LiveStore` kills and waits on drop, skips fail under `CI`, all paths are temp directories.

**T1. Medium — the first board keeps running into the second launch.** `src/runtime/tests/startup_smoke.rs:143-202` (`a_fresh_install_starts_against_a_real_store`, `drop(first)` at 176). *Verified* mechanism, flake *Plausible.* `src/runtime/mod.rs::start_background_tasks` (around line 932) drops the join handles of the row pump and the shared-store reconnect loop, which detaches them; nothing implements `Drop` for `Bootstrap` or `TuiRuntime`. The second launch reuses the same address and data directory, so the first board's reconnect loop can attach to the new store. Fix: make the runtime's background tasks abort on drop (keep `AbortHandle`s), or give the second launch a fresh address.

**T2. Low — port picking races, and the stop can hit a sibling's store.** `src/runtime/tests/startup_smoke.rs:33-38` (`free_address`) and `live_store.rs:45-48` (`LiveStore::start`) bind port 0, drop it, then hand it out. *Verified* mechanism, low probability. `LiveStore::start` probes before checking its own child, so it can accept another test's store, and `managed_store.rs:862` `stop(scan_port=true)` sends SIGTERM to every PID on the port. Fix: check `child.try_wait()` before trusting the probe; retry on port-in-use. Fold into T1.

**T3. Low — the previous-store gate never runs in CI.** `scripts/check-previous-store.sh:28-37` exits 0 with no local `main` or in a shallow clone, which is what CI checks out. *Verified.* The skip is documented; the gate runs only in a developer's pre-push hook. Fix: fetch `main` with depth in the CI job that runs it, or say plainly in `docs/testing.md` that CI does not enforce it. Fold into T1.

### 2.6 Test coverage

6,589 tests pass across 32 targets, 2 ignored, no `skipping:` lines (`tmux` and `spacetime` on `PATH`). Coverage not re-measured (last: 92.17%, 2026-10-15). Gaps worth closing come with their findings: a feed move out of My Reviews (N1), a tag-character display name (N2), `resume_task` on another host's task (N3), `v` with a failing content mark (G1), a combined relink + illegal patch (S1), and a CLI pin refusal's message (P5).

### 2.7 Complexity hotspots

Largest non-test, non-generated files are unchanged: `src/tmux.rs` (1,594), `src/dispatch/mock_sequence.rs` (1,531, test-support), `src/dispatch/worktree.rs` (1,502), `src/keybindings/table.rs` (1,441, a data table), `src/runtime/tasks.rs` (1,254), `src/store/mod.rs` (1,252), `src/runtime/mod.rs` (1,240). The generated `src/spacetime/bindings/mod.rs` (2,781) holds 214 of the 230 `#[allow]`s. In the module, `tasks_epics.rs` (1,091) and `session_request.rs` (1,072) are the largest. Only one `clippy::too_many_arguments` allow remains (`src/sync/encode.rs:309`). No new hotspot this week.

### 2.8 CLAUDE.md improvements

CLAUDE.md shrank (c9c5033a moved toolchain detail to `docs/testing.md`), which is the right direction. Two small suggestions, both for `docs/` rather than CLAUDE.md itself:

1. **Feed moves are epic changes.** `docs/invariants.md` could say that any write changing a task's `epic_id`, including feed routing, must end its unseen marks — N1 slipped because `IngestStore::update_task` looks like a plain row write.
2. **Bounded git calls must not prompt.** Once P1 is fixed, one line next to `run_bounded` in `docs/conventions.md`.

## 3. Status of the 2026-10-23 findings

| # | Finding | Status | Evidence |
|---|---|---|---|
| A1 | `run_bounded` kills only the child | **Fixed** | 22de00e0; `src/process.rs::run_bounded`, `::abandon`; residue P1 |
| A2 | Two syncs of one repo at once | **Partly fixed** | 2c589890; `App::repo_syncs_in_flight`; residue P2 |
| A3 | Killed merge left half-done | **Fixed** | 1f17ac33; `src/repo_sync.rs::merge_origin_base` |
| A4 | `measure_repo` and `~` | **Fixed** | `src/git.rs::detect_default_branch` expands `~` |
| C1 | Cron search window too short | **Fixed** | c4863833; `SEARCH_DAYS = 366 * 40` |
| C2 | Step add overflow | **Fixed** | `schedule.rs::parse_field` `checked_add` |
| C3 | Twin leaves a sub-epic on refusal | **Fixed** | `validate_task_write` arms creates |
| C4 | Phoenix back to backlog not re-armed | **Fixed** | `schedule.rs::rearm_phoenix_on_entering_backlog` |
| C5 | Stale schedule state on Done rows | **Fixed** | `session_request.rs::apply_session_close` |
| G1 | Editor key throws away edits | **Fixed (decision)** | 01eeb241, task 98620; residue G1 (10-28) |
| G2 | Editor flag in memory only | **Fixed** | `diff_pane.rs::PANE_CONTENT_OPTION` |
| G3 | No editor-flag tests | **Fixed** | `diff_pane.rs` tests |
| W1 | `learning_verdicts` not in snapshot | **Fixed** | a2c04027, d3d5a98f |
| W2 | Unowned repo paths never pruned | **Open (optional)** | `src/sync/rows.rs::repo_paths_saved_by` |
| F1 | Header-shaped lines eat the description | **Partly fixed** | `parse_sections_in`; residue P3, P4 |
| F2 | `scope_ref` not normalised | **Fixed** | `learnings.rs::normalise_repo_scope_ref` |
| F3 | One panic disables embeddings | **Fixed** | `embeddings.rs::serve_requests` `catch_unwind` |
| F4 | CLI skips the store pin | **Fixed** | `open_cli_store` checks the pin; residue P5 |
| C2 (10-22) | `update_task` relink ordering | **Open** | S1 |
| S2 (10-22) | `PaneId` newtype | **Open** | No `PaneId` in `src/` |
| — | Trust model | **Open (decision)** | No hits outside `docs/plans/` |

## 4. Prioritised action items

### Magic wand (top 3 by impact)

1. **Move the last task-write rules into the module** — S1 and S2 in `src/service/tasks/crud.rs::TaskService::update_task`. One reducer that takes epic, patch and the Done transition removes a two-write partial failure and a stale-cache decision at once.
2. **Decide and write down the store's trust model** — open across five reviews; N2 (what a display name may hide) and N3 (who may resume whose task) are two more faces of it.
3. **`PaneId` newtype** — `src/tmux.rs`, `src/dispatch/split_panes.rs`, `src/agent_tree/diff_pane.rs`. G3 (a second `$TMUX_PANE` parser) is exactly the drift a type would stop.

### Quick wins

- G6 — two stale doc comments.
- P3 — trim single-line editor fields.
- P5 — CLI-specific pin refusal message.
- S3 — the stale `delete_epic` doc comment.

### Work packages

- **WP-feed-and-profile-rules:** N1, N2 (`spacetime/module/src/feed_ingest.rs`, `user_profile.rs`, twin, conformance tests; rebuild `module.wasm`).
- **WP-resume-task:** N3 (`src/mcp/handlers/tasks/resume.rs`, `src/tui/update/agent.rs`).
- **WP-agent-tree:** G1–G6 (`src/agent_tree/`).
- **WP-task-write-rules:** S1, S2, S3 (`src/service/tasks/crud.rs`, `src/service/epics.rs`, module reducer and twin; `RULES_VERSION` bump).
- **WP-startup-smoke:** T1, T2, T3 (`src/runtime/`, `src/runtime/tests/startup_smoke.rs`, `scripts/check-previous-store.sh`).
- **WP-cli-and-git:** P1–P5 (`src/process.rs`, `src/dispatch/finish.rs`, `src/editor.rs`, `src/store_connection/mod.rs`).

### Decisions needed

- The store's trust model.
- N3, only if the spec should change instead of the code.
- W2 (optional): adopt unowned repo paths on prune.

## Test and lint run

`cargo test --no-fail-fast`: 32 targets, 6,589 passed, 0 failed, 2 ignored, no `skipping:` lines. `cargo clippy --all-targets -- -D warnings`: clean.
