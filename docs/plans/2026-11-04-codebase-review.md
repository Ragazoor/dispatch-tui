# Codebase review — 2026-11-04

Task #98864. Follows `docs/plans/2026-11-03-codebase-review.md` (task #98753).

**Range.** `071ba533..8687f9d4` on `main`, 32 commits (no merges). Main themes: the 2026-11-03 work packages (store publisher claim, feed-command setter rule and claims that follow sight, tasks stay in their workspace, raw-row compare for agents and poll claims, feed runner fixes, atomic setup writes, repo-path ownership, one resume per task, Team Reviews hints, the graduated checks, S2/T2), cross-host feed requests (247dddce), keep-awake (bec9e4b2), and nested epics as a tree in flattened columns and the agent pane (35da929d and follow-ups). The next run starts at `8687f9d4`.

**How.** Third run of `.claude/skills/repo-health-review`. Five read-only reviewers: (1) status and regression audit of the module and sync fix commits; (2) status and regression audit of the feed, setup, launch, repo-path and check fix commits, including the graduated checks; (3) a defect hunt over 247dddce alone; (4) a defect hunt over keep-awake and the epic tree; (5) the fresh area, `src/hooks/`, `src/tmux.rs` and the plugin hook scripts. I re-read the code for C1, Q1, N1, M12 and H1, and ran the gates.

**Gates (on `8687f9d4`).**
- `cargo test --no-fail-fast`: 32 targets, 6,802 passed, 0 failed, 2 ignored, no `skipping:` lines.
- `cargo clippy --all-targets -- -D warnings`: **failed** — `clippy::large_enum_variant` on `src/runtime/event_loop.rs::LoopEvent`, plus an `unused_mut` in `tests/memory_caller_conformance/move_epic.rs`. Fixed in this task (C1, commit `fix(tui): box Epic payloads on the message bus`); clippy is clean after it.
- The toolchain on this machine is Fedora's rustc/clippy 1.99.1 (installed 2026-11-01, no rustup), not the pinned 1.99.0, so `scripts/check-toolchain.sh` fails here. The same toolchain was clean on 2026-11-03, so C1 is a real regression, not toolchain drift. `scripts/check-spacetime-module.sh` fails here on a link error in `spacetime/module-tests` (missing `openssl-devel`/`gcc-c++`); environment, not code.
- Coverage not re-measured.

**Confidence key.** *Verified*: I re-read it in the code. *Verified by the reviewer*: a reviewer re-read it, I did not. *Plausible*: the path exists but the outcome depends on environment or timing.

## 1. Executive summary

- **The 2026-11-03 work packages landed: 41 of 51 items closed** (Fixed, Settled, Moot or an accepted gap), 6 partly fixed, 4 "also noted" items untouched. S2 is finally closed; T2 is only partly closed. Several closed items left residues (M4, Y4, M1).
- **`main` failed clippy** (C1). 94a6acdc added `Epic::feed_setter`, which pushed the inline `Epic` payloads in `EpicMessage` over `large_enum_variant`. The commits reached `main` by wrap-up rebase, which runs no pre-push hook, and the repo's verify command has no clippy step. This is the second time (2026-10-14 was the first): it graduates into a check (C2).
- **One High in new code.** Cross-host feed teardown snapshots `task.tmux_window`, a column the store always blanks, so the holding host never kills the agent's window (Q1).
- **Fixes that regressed:** the Team Reviews hint on stderr turns every PR Reviews cycle additive under the default config, so stale review cards are never removed (N1); claims held before the setter rule stall a shared feed for good (M9); request rows now arrive but the task they name does not, so a followed-then-unfollowed task's wrap-up fails (M12).
- **Fresh area (`src/hooks/`, `src/tmux.rs`):** a hook picks its task id from the current branch name, so an agent on another branch can flip a different task to Review (H1); a failed resume leaves an empty window that the next resume "reattaches" to (H2); tmux calls the #4202 bound was meant to cover still run unbounded (H3, H4).

## 2. Status of the 2026-11-03 findings

| # | Finding | Status | Where |
|---|---|---|---|
| M1 | Restore broken on old stores; first-come publisher | Fixed, window documented | d5341163: `managed_store.rs::SpacetimeManagedStore::publish_embedded_module` claims after publish; `claim_store_owner` cannot be gated, recorded as a KNOWN WINDOW in `spacetime-seed.allium`. No test imports into an older store. Claim runs only on a module change (M13) |
| M2 | Any member makes a teammate's host run a command | Fixed | 94a6acdc: `feed_command_set_by` stamped at create, patch and managed roots; `host_may_run_feed` on auto, manual and request paths; sight required in claim/override; poll-owner arm gone. Legacy claims stall (M9) |
| M3 | Detach hides a teammate's task | Fixed (stricter than offered) | caafe9ea `visibility.rs::task_move_leaves_workspace`: only the operator may detach. Spec and TUI left behind (M11) |
| M4 | Three paths strand poll claims | Fixed for epic scope | 94a6acdc: all five loss-of-sight paths release. Task-scoped claims not released (M10) |
| M5 | `TaskPatch.owner` from the client | Fixed | caafe9ea `rules.rs::check_task_patch` |
| M6 | Twin `update_epic` partial write | Fixed | caafe9ea dry-runs in the module's order |
| M7 | Restore reducers ungated | Fixed | d5341163 `require_restore_authority` |
| M8 | Conformance gaps | **Partly fixed** | Subscribe refusals covered; `created_by` stamp and `ReParentingKeepsTheHolder` conformance cases still missing |
| Y1 | No rules-version bump | Fixed | Both constants at 35; bump rule added to the `RULES_VERSION` doc |
| Y2 | Agents and claims false-erased | Fixed | a9f80e70 raw compare in `rows.rs::retire_agent`, `::retire_poll_owner` |
| Y3 | Requests vanish when a task stops being held | **Partly fixed** | a9f80e70 `views.rs::rows_on_owned_hosts`; the task itself still vanishes (M12) |
| Y4 | Undecodable-to-undecodable erases the mark | Fixed for tasks and epics | a9f80e70; sibling error branches keep stale rows (M15) |
| Y5 | Twin ignores the views | Accepted gap | Documented in `docs/testing.md` |
| U1 | Two resumes, two `claude --continue` | Fixed | ac34b537 `App.resuming` |
| U2 | Team Reviews empties on upgrade | Fixed, regression | 7d45d991 → N1 |
| U3 | `CARGO_BUILD_JOBS` overrides `build.jobs` | Fixed | ac34b537; misses the inline `build = { jobs = 2 }` form (N7) |
| U4 | Scope check vs launch environment | Fixed | ac34b537 probe-and-fall-back in the pane |
| U5 | bash 3.2 hook abort | Fixed | 7d45d991 |
| U6 | Space cannot remove a hide | Fixed | 2a06ce51 |
| U7 | Resume wait never times out | Fixed | ac34b537 `RESUME_JUMP_MAX_READS` |
| U8 | Scope and sccache outside the deadline | **Partly fixed** | ac34b537 skips past the deadline, but the first `prepare_agent_scope` still runs unbounded (N8) |
| F1 | Teardown ignores the owning host | **Partly fixed** | ab119f64 + 247dddce; another person's host gets no row (KNOWN GAP in `store-split.allium`); window never killed (Q1) |
| F2 | Repo paths against other hosts' clones | Fixed, regression | dd8e17a1 → N3 |
| F3 | Feed cycles serialised per process | Fixed | c005d8d0 + 247dddce (owner serves refresh requests); `ingest_feed` still does not check poll ownership (Q9) |
| F4 | Setup rewrites live scripts in place | Fixed | 92d3085b |
| F5 | `~/.claude.json` lost updates | Fixed (window narrowed) | 92d3085b re-read and retry |
| F6 | Silent failing feed | Fixed | 34808ee0 |
| F7 | Below-floor log spam | Fixed | 9065b148 |
| K1 | Prune deletes another host's row | Fixed | 942e92c0 adopt, never delete |
| K2 | Module re-stamps the host | Fixed, regressions | 942e92c0 → N2, N3 |
| K3 | ssh without BatchMode | Fixed, regression | 2a06ce51 `setsid` → N4 |
| K4 | Refused create moves the cursor | Fixed | 2a06ce51 |
| G1 | Move-path guard inverted | Fixed, blind spots | 8c865325 (G3) |
| G2 | Single-write check misses wrapped calls | **Partly fixed** | 8c865325 (G4) |
| S2 residue (standing) | `close_pr_on_done` reads the cache | **Fixed** | 6220f900: the module's `update_task` writes a close row (`UpdateTaskDoneWritesACloseRow`) |
| T2 residue (standing) | `free_address` binds and drops | **Partly fixed** | 6220f900 fixed `live_store.rs`; `managed_store_real.rs::free_address` and `startup_smoke.rs` still bind and drop |
| 3.7 spec items (7) | | Settled | 2a06ce51, c005d8d0, c0e7f49c, 9065b148, caafe9ea |
| 3.8 `--body` leading dash | | Fixed | 7d45d991 |
| 3.8 `require_epic_accepting_work` | | Fixed | 2a06ce51 removed |
| 3.8 detached agents, `is_address_in_use` | | Moot | |
| 3.8 group kill only on timeout; old repo groups; non-owner dump test; hot-swap private table | | Open | Not touched |

Yield: 41 of 51 closed; 6 partly (M8, Y3, U8, F1, G2, T2); 4 "also noted" items open. M1, M4 and Y4 are closed with residues (M13, M10, M15).

## 3. Findings

Every module finding with a twin counterpart must be mirrored in `src/sync/memory_caller/` and get a case in `tests/memory_caller_conformance/`.

### 3.1 Gates

**C1. High — fixed in this task — `main` failed `cargo clippy --all-targets -- -D warnings`.** `src/tui/messages/epic.rs::EpicMessage` (`Updated`, `Created`, `Edited` carried `Epic` inline), `src/runtime/event_loop.rs::LoopEvent`; `tests/memory_caller_conformance/move_epic.rs` (`let mut make_epic`, from caafe9ea). *Verified.* 94a6acdc added `Epic::feed_setter` (24 bytes); `EpicMessage` reached 240 bytes and `LoopEvent` tripped `large_enum_variant`. The bus-size guard `src/tui/types/tests.rs::assert_no_entity_inline` compared against `Task` only; `Task` is larger than `Epic`, so the inline `Epic` passed. Fix applied: box the three payloads (as `TaskMessage::Updated(Box<Task>)`), bound the guard by the smaller of `Task` and `Epic`, drop the `mut`.

**C2. Medium — graduate — wrap-up rebases put commits on `main` that no lint gate has seen.** Task verify command (`get_task`: `cargo fmt --check && cargo test && ./scripts/check-doc-paths.sh`), `.githooks/pre-push`. *Verified.* The pre-push hook runs clippy and every `check-*.sh`, but a rebase-mode wrap-up lands commits on `main` without a push, and the verify command runs neither clippy nor the other checks. Second occurrence of the class (2026-10-14: `check-no-test-sleep.sh` and `check-rustdoc-links.sh` failed on committed `main`; now clippy). CLAUDE.md's prose rule ("run the hook's check scripts before committing") did not hold. Fix: one script, `scripts/verify.sh`, that runs what the pre-push hook runs plus the tests, used both as the repo's verify command (`set_verify_command`) and by the hook, so the two cannot drift. Then delete the prose rule from CLAUDE.md.

### 3.2 Cross-host feed requests (247dddce)

**Q1. High — teardown rows never name the agent's window.** `spacetime/module/src/feed_request.rs::new_teardown_request`, `::teardown_request_owed`; `feed.rs::delete_stale_feed_tasks_in_epic`. *Verified.* The snapshot copies `task.tmux_window`, but `agent.rs::without_session_facts` blanks that column on every task write; the window lives on the current agent row. The row is also written after `delete_task_side_effects`, which deletes the agents. Scenario: host A2 polls; feed task T runs on A1 in window `dispatch:task-123`; the item leaves the feed; the row reaches A1 with an empty window; A1 removes the worktree and the agent keeps running in a deleted directory. A task with a window and no worktree owes no row at all. `store-split.allium: StaleDeleteRequestsTeardownOnTheHoldingHost` says `current_agent(task).tmux_window`. Tests pass because `spacetime/module-tests/tests/feed_requests.rs::hold` writes `tmux_window` on the task row, which no real path does. Fix: read the window from the agent tables before `delete_task_side_effects` and pass it in; mirror in `src/sync/memory_caller/feed_requests.rs`; set the window on an agent row in the test.

**Q2. Medium — anyone who sees a task can aim another host's teardown at an unrelated worktree.** `feed_request.rs::write_teardown_request_if_owed`; `tasks_epics.rs::patch_task`, `rules.rs::check_task_patch`. *Verified by the reviewer.* The snapshot takes `host`, `repo_path` and `worktree` from the task row, and `TaskPatch` lets any viewer set all three. Scenario: Bob patches a feed task in Alice's epic to `host = A1`, `worktree = /home/alice/proj/.worktrees/999-other`; when the item leaves, A1 runs `git worktree remove --force` and `git branch -D` on a live worktree. Same class as M2: another person makes your host act. Fix: refuse `host`/`repo_path`/`worktree` patches unless the caller acts for the task's host, or snapshot from the module-stamped dispatch request row instead.

**Q3. Medium — a failed claim call drops a request for the life of the board.** `src/feed/requests.rs::rows_to_serve`, `src/feed/mod.rs::serve_feed_requests`. *Verified by the reviewer.* The id enters `served_requests` before `claim_feed_request` returns; on `Err` (transport, reconnect) the row stays `requested` and is never looked at again. Teardown rows have no backstop and are not pruned while open, so the worktree and window stay until a board restart. Same for a `running` row whose done/failed report fails. The set is never trimmed. Fix: insert only after `Ok(true)`; drop ids whose row is no longer open.

**Q4. Medium (plausible) — two boards on one host id can both serve a request.** `feed_request.rs::validate_feed_request_claim`. Re-claiming a `running` row is always accepted. Two boards sharing a host id both run the teardown; one reports `failed`, the other's `done` is refused, and the row ends `failed`. Fix: allow a re-claim only after a grace period on `claimed_at`, or require a per-process claim token on report.

**Q5. Low — `store-split.allium` contradicts itself on pruning.** `RequestRowsArePruned` guidance says an ingest "prunes nothing"; `StaleDeleteRequestsTeardownOnTheHoldingHost` and the code (`write_teardown_request_if_owed` calls `prune_feed_requests`) prune. The open questions "Feed teardown rows a board never serves" and "prune emitters" are answered by the code but still listed open. *Verified by the reviewer.* Fix: reconcile the text and close the questions.

**Q6. Low — a foreign-owner teardown is logged at debug as written.** `src/feed/cycle.rs::FeedCycle::run`. *Verified by the reviewer.* For a task on another person's host the module writes no row (KNOWN GAP), but the log, now `debug`, says "a teardown request was written". Their window and worktree are stranded silently. Fix: keep a `warn` when the holding host's owner is not this board's identity, and word it truthfully.

**Q7. Low — the twin's `request_feed_refresh` skips the sight check.** `src/sync/memory_caller/feed_requests.rs::request_feed_refresh_body` reads `tables.epics` directly; the module refuses an unseen epic. *Verified by the reviewer.* Fix: use the twin's visible-epic helper; add a conformance case.

**Q8. Low — a second `r` after the owner already ran is answered "Requested" but nothing re-runs.** `feed_request.rs::request_feed_refresh` coalesces into a `running` row. *Verified by the reviewer.* Fix: coalesce only into a `requested` row.

**Q9. Low — `ingest_feed` does not check poll ownership.** `spacetime/module/src/feed_ingest.rs::ingest_feed` checks only `require_acting_for(host)`. A host whose claim is overridden mid-cycle still reconciles, overlapping the new owner. *Verified by the reviewer.* Fix: refuse an ingest from a host that does not hold the epic's claim (or say in `feeds.allium` why it is safe).

### 3.3 Module and sync: regressions in last round's fixes

**M9. Medium — fix for M2 → claims held before the setter rule stall shared feeds for good.** `src/models/poll_action.rs::decide_feed_poll_action`, `spacetime/module/src/agent_state.rs::check_feed_claim_by`, `src/feed/mod.rs::FeedRunner::this_host_polls`. *Verified by the reviewer.* Only new claims are gated. Scenario: Alice set a shared feed's command; Carol's host claimed it before rules 32. After the upgrade Carol's host may not run it and skips; Alice's host sees a claim by Carol and skips too. The feed stops with no notice; only a manual override recovers it. Fix: treat a claim whose host owner is not the setter as absent (the setter's claim replaces it), or sweep such claims once; state it under `feeds.allium: FeedCommandRunsOnSettersHost`.

**M10. Low–Medium — fix for M4 → task-scoped poll claims still outlive sight.** `spacetime/module/src/config.rs::release_poll_claims` filters to epic scope, while 94a6acdc makes a task claim require `visible_task`. *Verified by the reviewer.* Scenario: Carol holds task T's PR-poll claim and is removed from the workspace; nobody else may claim T, and its PR status stops updating. Fix: release task-scoped claims in `ClaimsFollowSight` over the affected tasks; mirror; add to both loss-of-sight tests.

**M11. Low — fix for M3 → spec and TUI still offer a detach that always fails.** `docs/specs/tasks.allium::MoveTaskToEpic` ("a null new_epic detaches…"), `src/tui/update/move_task.rs` ("— no parent —"), `workspaces.allium::ReParentingKeepsTheHolder` guidance. *Verified by the reviewer.* Pressing `m` → "— no parent —" on any task in an epic is refused with "a task stays in its workspace". caafe9ea chose "only the operator detaches", stricter than the three options last round offered; the commit cites #98852. Fix: update `MoveTaskToEpic`, hide the sentinel for a task in an epic, drop the unreachable arm.

**M12. Medium — fix for Y3 → the request arrives but its task does not.** `spacetime/module/src/views.rs::held_tasks_of` (own + covered only), `::subagents_of_held`; `src/service/tasks/session.rs::serve_rebase`, `src/service/tasks/dispatch.rs` (both `self.db.get_task`). *Verified.* Scenario: I dispatch a teammate's task from an epic I follow, then unfollow it; the agent calls `wrap_up`; the session request reaches my reactor, `get_task` answers None, and the rebase fails as "task deleted". Subagent rows of that task also disappear from the agent tree. Fix: serve tasks with a live agent or open request on a host the caller owns; same rule for `subagents_of_held`; a views test; state it in `sync.allium`.

**M13. Low — fix for M1 → the publisher claim runs only on a module change.** `src/spacetime/managed_store.rs` publishes, and so claims, only when `module_action` is `Publish`; a failed claim is a `warn` and the hash is recorded anyway. *Verified by the reviewer.* A transient failure leaves the store without a publisher until the next module change, though `spacetime-seed.allium` says the next start fixes it. Fix: claim on every managed bring-up (it is idempotent), or reword the spec.

**M14. Low — doc slips in 94a6acdc.** `src/tui/update/feeds.rs`: `handle_feed_not_authors_host`'s doc comment was inserted between `handle_feed_already_refreshing`'s doc and its `fn`. `feeds.allium::FeedCommandRunsOnSettersHost` cites `decide_poll_action`; the code is `decide_feed_poll_action`. *Verified by the reviewer.*

**M15. Low — Y4's sibling error branches keep stale rows.** `src/sync/rows.rs::upsert_learning`, `::upsert_learning_retrieval`, `::upsert_work_package`, `::upsert_work_package_link` keep the old decoded row on a decode error; a9f80e70 fixed only tasks and epics. *Verified by the reviewer.* Fix: remove the row in the error branch, as `upsert_task` does.

### 3.4 Feeds, repo paths and process: regressions in last round's fixes

**N1. Medium — fix for U2 → PR Reviews never removes a stale card under the default config.** `scripts/fetch-reviews.sh::search_team_prs` (the "TEAMS is empty" line on stderr), `scripts/teams.conf` (`TEAMS=()`), `src/feed/exec.rs::degraded_partial_emission`. *Verified.* Any stderr with items > 0 makes the cycle additive (`DegradedNonEmptyEmission`); with 0 items it fails. Every user who has not set teams gets a PR Reviews epic whose merged, approved and no-longer-requested cards are never removed, and a manual refresh reports "degraded" each time. `tests/feed_scripts.rs::fetch_reviews_hints_on_stderr_when_teams_conf_lists_no_teams` asserts the stderr without asking how the runner classifies it. Fix: move the hint off stderr (setup, the TUI, or a notice prefix the runner ignores); add a test that runs the default config through the emission classifier.

**N2. Low — fix for K2 → a TUI delete of another host's or a legacy repo path silently does nothing.** `src/runtime/tasks.rs::exec_delete_repo_path` → module `config.rs::delete_repo_path` (filters `host == caller's host`, returns Ok). *Verified by the reviewer.* The overlay lists every host's paths; the row comes back on reload. With K1 a legacy row whose path exists nowhere can never be removed. Fix: refuse with "owned by host X"; decide in `cli.allium`/`core.allium` who may remove an unowned row.

**N3. Medium (plausible) — decision: fix for K2 + F2 → a path shared by two machines resolves for feeds on only one.** `config.rs::save_repo_path` keeps the first saver's host; `feed/cycle.rs::sync_emission` uses `list_repo_paths_saved_by_this_host`. Scenario: laptop and desktop both have `~/Code/work/api`; the desktop saved first; when the laptop polls, `org/api` items get no repo path. `core.allium` says the first saver keeps it, so this is the user's call. Fix options: key `repo_paths` by (path, host), or let the feed fall back to rows whose path exists on this disk.

**N4. Low (plausible) — fix for K3 → `setsid` children can pop a GUI askpass.** `src/process.rs::detach_from_terminal`, `::non_interactive_env`; `src/feed/exec.rs::run_feed_command_bounded`. Without a tty and with `DISPLAY`/`WAYLAND_DISPLAY` set, OpenSSH uses askpass; 2a06ce51 also dropped `BatchMode=yes`. With an askpass helper installed, each bounded fetch with a passphrase key pops a dialog until the deadline kills it. Not reproducible on this machine (no askpass helper). Fix: set `SSH_ASKPASS_REQUIRE=never` for these children.

**N5. Low — G2's allow-list accepts a real partial write.** `src/service/managed_feeds.rs::write_managed_feed_settings` is allow-listed as "four independent settings rows", but its own comment says the command and interval are coupled; a failure after the first write leaves them inconsistent. *Verified by the reviewer.* Fix: one reducer, or an honest allow-list reason.

**N7. Low — U3's `build.jobs` detection misses the inline table form.** `src/dispatch/build_jobs.rs::cargo_config_sets_jobs` does not see `build = { jobs = 2 }`. *Verified by the reviewer.* Fix: parse the config as TOML.

**N8. Low — U8 residue: the first scope probe still runs past a short deadline.** `src/dispatch/request_host.rs::DeadlineRunner::agent_scope` delegates to `RealProcessRunner::agent_scope`, whose first call runs `prepare_agent_scope` (up to 3 × 10 s) on the unbounded runner; the sccache start is bounded at 5 s, not the time left. *Verified by the reviewer.* Fix: bound both by the remaining deadline.

### 3.5 The graduated checks (audited by mutation)

**G3. Low — the G1 guard still has blind spots.** `tests/memory_caller_conformance/move_paths.rs::epic_id_writers`. *Verified by the reviewer.* It catches a multi-line `Task { epic_id: … }` literal or an `.epic_id =` assignment, but misses: a one-line literal (`write_task(ctx, Task { epic_id, ..t })`, which rustfmt keeps on one line), a `pub(super) fn` or `async fn` (attributed to the previous function), module files in subdirectories (`read_dir` is not recursive), and a site added to an existing table row without being exercised. Fix: match `epic_id` anywhere in a `Task {…}` span, accept any visibility/async prefix, recurse, and map each `MOVE_PATHS` entry to exactly one site.

**G4. Low — `check-single-write-service.sh` still misses loops, helpers and rebinding.** *Verified by the reviewer* (ran a copy against probe files). Now caught: wrapped `self\n.db\n.m(`, free functions, an allow-listed function gaining a write. Still missed: writes in a loop (counted once), a helper that writes called twice, `let store = &self.db;` rebinding, a `pub(in …) fn` header, and anything outside `src/service/`. The loop and helper cases are the same "N writes for one change" class. Fix: count loop bodies as many; follow `self.<helper>()` one level; scan `src/runtime/` too. Or accept the limits in the script's header.

**G5. Medium — graduate — the two "demanded" tests from 2026-11-03 exist only in part.** *Verified by the reviewer.* The loss-of-sight test exists twice, separately (`spacetime/module-tests/tests/feed_authority.rs` and `src/sync/memory_caller/tests/feed_authority.rs`), not as one conformance test, though `workspaces.allium: ClaimsFollowSight` says "one conformance test runs each of them"; task-scoped claims are not in it (M10). The ignored-column test for every table behind a view does not exist: `src/sync/tests/view_updates.rs` covers tasks, epics, agents and poll owners only. Fix: move the loss-of-sight case into `tests/memory_caller_conformance/` so both sides run it, and extend `pins_view_updates!` to send a dropped-column update for every view-backed table (learnings' `embedding` included).

### 3.6 Keep-awake and the epic tree

**T1. Medium (plausible) — the board can miss the end of a GNOME hold.** `src/keep_awake.rs::start`. The reader waits for EOF on both pipes before `child.wait()`; the inner `sh -c 'echo ready; exec cat'` inherits them. `systemd-inhibit` kills its child on death; `gnome-session-inhibit` (as far as the reviewer knows) does not. If `gnome-session-inhibit` dies alone, `cat` holds the pipes open, `HoldEvent::Ended` never fires, and the "awake" badge stays with no hold (`BadgeMeansHeld`, `KeepAwakeLost`). Fix: race `child.wait()` against the reads, or spawn in its own process group and signal the group.

**T2. Low–Medium (plausible) — GNOME detection reads the tmux server's environment.** `src/keep_awake.rs::hold_command_for_this_machine`. A tmux server started from ssh or a TTY has no `XDG_CURRENT_DESKTOP`; on GNOME the board picks `systemd-inhibit --what=idle:sleep`, which GNOME ignores for lock and blanking, while the badge says "awake". The `OnceLock` keeps the wrong choice. Fix: also check `XDG_SESSION_DESKTOP`/`DESKTOP_SESSION` or `tmux show-environment -g`; or try `gnome-session-inhibit` first when it is on `PATH`.

**T3. Low — no escape from `Requesting`.** `src/tui/update/keep_awake.rs::handle_toggle_keep_awake`. A helper that neither prints `ready` nor exits (a polkit prompt, a hung D-Bus call) leaves the state `Requesting` and every `W` ignored. Fix, spec first (`keep-awake.allium`): let `W` cancel a request, or time it out into Refused.

**T4. Low — unplaced epics share a sort scale with placed ones.** `src/tui/types/layout.rs::EpicTree::build`, `FlattenedGroupKeys::key_for`. *Verified by the reviewer.* An epic on a parent cycle is never placed and falls back to `sort_key()` (id), which collides with placed ranks, interleaving groups into repeated headers. Needs corrupt data (`validate_epic_parent` refuses cycles). Fix: rank unplaced epics after all placed ones.

**T5. Low — scrolling up can hide a nested group's root header.** `src/agent_tree/render/agents.rs::render_agents` checks only the line above the cursor; a nested group draws several headers before its first agent. *Verified by the reviewer.* Fix: walk back over consecutive header lines; update the spec wording.

**T6. Low — `handle_pane_view` is quadratic in epics.** `src/mcp/handlers/hooks.rs::handle_pane_view` calls `models::epics::epic_path`, which rebuilds `epic_id_lookup` each time, once per agent epic, on every pane poll, though `by_id` is built a few lines above. *Verified by the reviewer.* Fix: an `epic_path_with(target, &by_id)` variant.

### 3.7 Fresh area: `src/hooks/`, `src/tmux.rs`, plugin hook scripts

**H1. Medium-High — a hook takes its task id from the branch name, and the board trusts it.** `plugin/hooks/scripts/task-status-hook`, `plugin/hooks/scripts/pr-learnings-hook` (`git branch --show-current | grep -oP '^\d+'`), `src/hooks/mod.rs::run_event`, `src/mcp/handlers/hooks.rs::apply`. *Verified.* The `/hook` request carries no caller identity. Scenario: the agent on task 42 runs `git checkout -b 7-hotfix`, or `cd`s into a checkout whose branch starts with digits; its next Stop runs `try_record_stop(7)` and flips task 7 to Review while task 7's agent works; SessionStart clears task 7's subagents; a `gh pr create` spends task 7's PR-gate reminder. Ids are global, so task 7 can be on another host. `agent-health.allium: HookDelivery` says the hook resolves from the session's worktree. Fix: put `DISPATCH_TASK_ID` in `dispatch::agent_launch_env`, prefer it in both scripts, fall back to the branch; optionally check the payload's `session_id` against the task's current agent. Add a hook-script test for a cwd on another task's branch.

**H2. Medium — a resume that fails after creating the window leaves an empty window the next resume adopts.** `src/dispatch/agents.rs::resume_agent_with_message`, `src/tui/update/agent.rs::handle_resume_failed`. *Verified by the reviewer.* If `set_window_dispatch_dir`, `ensure_split_hook`, the prompt write or `send_keys` fails after `new_window`, the window stays with a bare shell; the next resume finds it via `has_window`, "reattaches", marks the task Running, and drops the message. The fresh-dispatch path rolls back with `rollback_failed_provisioning`. Fix: kill the window on any error after `new_window`; add a test for a failure after the window exists.

**H3. Medium — the #4202 subprocess bound is incomplete.** `src/tmux.rs::new_window` (`refuse_duplicate_window_name` → `has_window` → `list_all`, an unbounded `list-windows -a`), `::set_window_option` (`window_target`, an unbounded `list-panes -a`), and `send_keys`, `select_window`, `kill_window`, `join_pane`, `respawn_pane*`, `split_*`. *Verified by the reviewer.* `run_checked_timeout`'s doc says it "closes #4202", but a wedged tmux server still parks provisioning and resume threads; the provisioning watchdog that once capped this (`PROVISION_MAX_SUBPROCESS_CALLS`, d0c2fe26) is gone. Fix: bound `list_all` and `window_target`, or make `run_checked` bounded by default.

**H4. Medium — jumping to a task window runs tmux on the command-drain thread.** `src/runtime/split.rs::exec_jump_to_tmux` calls `tmux::select_window` inline (an unbounded `list-panes -a` plus `select-window`). *Verified by the reviewer.* `src/runtime/editor.rs` states the rule this breaks: a wedged server "would otherwise freeze the command-drain thread, and with it input and rendering". Fix: `spawn_blocking` and report through `msg_tx`, as the sibling `exec_*` helpers do.

**H5. Low-Medium — entering split mode targets the session's active pane.** `src/runtime/split.rs::enter_split_outcome`, `::enter_split_with_task_outcome` → `tmux::current_pane_id` (untargeted `display-message -p`). *Verified by the reviewer.* With grouped sessions, or the operator viewing the board from another client, the split or join lands in the wrong window and the recorded "dispatch pane" is not the board. Fix: `tmux::self_pane_id()` read at the entry point, as `setup_tmux_for_tui` does.

**H6. Low — teardown restores the board window's name by a server-wide lookup.** `src/tmux.rs::rename_window_in_session` resolves `target` through server-wide `window_target`; `src/runtime/mod.rs::teardown_tmux_for_tui` passes "TUI". *Verified by the reviewer.* With two boards in two sessions the lookup fails ("multiple windows named 'TUI'") and the name is never restored, or the other session's window is renamed. The call-site comment claims the opposite. Fix: target `self_pane_id()`.

**H7. Low — hook scripts need GNU grep and abort early on a slow board.** Both plugin hook scripts. *Verified by the reviewer.* `grep -oP` fails silently without `-P` (macOS), so every branch lookup is empty. In `task-status-hook`, `set -e` aborts when `dispatch hook … pre_tool_use` fails, so the AskUserQuestion notification and the SendMessage observer never run. Fix: `grep -oE '^[0-9]+'`; guard the board call with `|| true` where later steps must still run.

**H8. Low — `~/.tmux.conf` is rewritten in place, and a commented-out line counts as set.** `src/tmux.rs::write_focus_events_to_tmux_conf_at`, `::contains_focus_events`. *Verified by the reviewer.* `fs::write` truncates then writes; `# set -g focus-events on` matches the substring check. Fix: `crate::atomic_write`; match an uncommented line.

**H9. Low — doc drift in `src/tmux.rs`.** `split_window_below_running` says the split hook "types `cd <dir>`" (it uses `respawn-pane -k -c`); `refuse_duplicate_window_name_in_session` claims the server-wide version's soft-fail (it errors); `run_checked_timeout`'s user list omits `set_window_awaits_pane` and `rejoin_pane`. *Verified by the reviewer.*

**H10. Low — the peer-message audit append is detached and unchecked.** `src/mcp/handlers/hooks.rs::append_peer_message_trajectory` runs in a detached `tokio::spawn`; its failure is swallowed and the hook reports `Applied`. *Verified by the reviewer.* Fix: say "best-effort" in `agent-health.allium: HookPeerMessageSent`, or await and log.

### 3.8 Also noted (not filed)

- `run_feed_command_bounded` still waits for stdout EOF; a background child holding stdout turns a valid emission into a timeout (carried from 2026-11-03, plausible).
- `src/feed/cycle.rs::sync_emission` now adds blocking `git remote get-url` calls (10 s each) inside an async function (dd8e17a1).
- `src/sync/memory_caller/tasks_epics.rs::apply_update_epic` still discards each part's outcome in the apply phase; safe while the dry run matches.
- `scripts/check-commit-msg.sh` exits non-zero without an argument, so a loop over `scripts/check-*.sh` reads it as a failure.
- Carried, not touched: repo groups made private before b28d2261 are not migrated; no test of a non-owner `SpacetimeCliStore::rows` dump; a connected old board after a hot-swap made a table private.

## 4. Hotspots and agent context

**Hotspots** (`git log --since=60.days -- src spacetime/module/src`; change count, current lines).

| File | Changes | Lines |
|---|---|---|
| src/spacetime/module.wasm.source-hash | 100 | 1 |
| src/spacetime/module.wasm | 92 | binary |
| src/runtime/mod.rs | 70 | 1,264 |
| spacetime/module/src/lib.rs | 57 | 333 |
| src/spacetime/bindings/mod.rs | 56 | 3,154 (generated) |
| src/service/tasks/crud.rs | 56 | 913 |
| src/store/mod.rs | 55 | 1,442 |
| src/sync/writes.rs | 54 | 764 |
| src/tui/mod.rs | 52 | 939 |
| src/runtime/tasks.rs | 49 | 1,272 |
| src/tui/types.rs | 44 | 585 |
| src/sync/rows.rs | 43 | 1,368 |
| src/mcp/handlers/tests/mod.rs | 43 | 1,699 |
| src/mcp/handlers/dispatch.rs | 43 | 1,058 |
| src/db/mod.rs | 43 | removed |
| spacetime/module/src/rules.rs | 42 | 493 |
| src/sync/tests/writes.rs | 40 | removed |
| src/sync/memory_caller/mod.rs | 40 | 1,042 |
| src/runtime/tests/mod.rs | 40 | 973 |
| src/models/tasks.rs | 40 | 1,178 |

Changes since 2026-11-03: `spacetime/module/src/rules.rs` and `src/sync/memory_caller/mod.rs` entered; `src/runtime/tests/misc.rs` and `src/main.rs` left. In-range diffs of the busy-and-large five: `crud.rs` (−61/+12) removed `close_pr_on_done` and `pr_url_to_close`; ClosePrOnDone now lives in the module (S2 closed). `src/store/mod.rs` (+104) adds `FeedRequestStore` and `adopt_repo_path`, and the test-only seed helpers. `src/sync/rows.rs` (+121/−32) is the Y2/Y4 raw-row work (M15 for its siblings). `src/sync/writes.rs` (+33) adds the feed-request reducers and the `host` on `update_task`/`ingest_feed`. `src/runtime/mod.rs` and `src/tui/mod.rs` changed 6 and 12 lines (keep-awake wiring); no finding.

**Agent context** (`wc -w`; 2026-11-03 → now): `CLAUDE.md` 2,281 → 2,281; `docs/invariants.md` 1,486 → 1,486; `docs/conventions.md` 13,096 → 13,096; `docs/reference.md` 9,874 → 10,051; `docs/testing.md` 4,501 → 4,648; `docs/module-map.md` 4,938 → 4,987; `docs/architecture.md` 2,035; `docs/how-to.md` 1,784; `docs/mcp.md` 2,836; `docs/mcp-tool-design.md` 875. All `CLAUDE.md` + `docs/*.md`: 43,706 → 44,079 (+0.9%). Flat; no action.

Claims checked against code: `serve_dispatch_request` is called only from `src/runtime/dispatch_host.rs`, `serve_session_request` only from `src/runtime/session_host.rs`; `report_pr_status`, `report_pr_unreadable`, `request_wrap_up`, `request_exit_session` exist in `src/service/`; `BOARD_RULES_VERSION` is tied to the module's `RULES_VERSION` by a test in `src/sync/rules_gate.rs`; `unwrap_used`/`expect_used` are `warn` in `Cargo.toml`. All hold. Prose that a mechanism should enforce: CLAUDE.md's "run the hook's `check-*.sh` scripts before committing" (C2 replaces it with the verify command).

## 5. Standing items

- **T2 residue** — `free_address` binds and drops in `managed_store_real.rs` and `startup_smoke.rs`. Raised 10-28, 10-31, 11-03 and now; task #98860 closed Done with `live_store.rs` fixed only. **Ask the user:** file again as its own package, or accept the retry loop.
- **N3** — one row per repo path vs one per (path, host). A design decision in `core.allium`.
- **M11's decision** — caafe9ea made "only the operator detaches" the rule. Confirm that is the intent before the TUI and spec are aligned to it.

**Graduate to a check.**
- "Commits reach `main` through a rebase with no lint gate" (2026-10-14, now C1): the verify command (C2).
- "A sibling path forgets the rule its fix added" appeared again: M10 (task scope), M12 (`subagents_of_held`, the task itself), M15 (sibling error branches), Q7 (twin sight). The checks meant for it now catch more but still have holes (G3, G4), and the two demanded tests are partial (G5). The deliverable stays the mechanical tests in G5, not more one-off fixes.

## 6. Areas read

- Every diff in `071ba533..8687f9d4`; 247dddce per file.
- In full or near full: `spacetime/module/src/{feed_request,feed,agent_state,visibility,views,config,tasks_epics,rules}.rs` and their twins in `src/sync/memory_caller/`; `src/sync/rows.rs`; `src/feed/{mod,cycle,requests,exec}.rs`; `src/keep_awake.rs`, `src/tui/update/keep_awake.rs`; `src/tui/types/layout.rs`; `src/agent_tree/render/agents.rs`; `scripts/check-single-write-service.sh`, `scripts/fetch-reviews.sh`; `tests/memory_caller_conformance/move_paths.rs`.
- Fresh area: `src/hooks/` (`mod.rs`, `wire.rs`), `src/tmux.rs`, `plugin/hooks/hooks.json`, `plugin/hooks/scripts/task-status-hook`, `plugin/hooks/scripts/pr-learnings-hook`, `src/mcp/handlers/hooks.rs`; oracles `agent-health.allium` (HookDelivery, HookStop).
