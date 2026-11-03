# Codebase review — 2026-11-03

Task #98753. Follows `docs/plans/2026-10-31-codebase-review.md` (task #98742).

**Range.** `e68f86a1..071ba533` on `main`, 47 commits (no merges). Main themes: the 2026-10-31 work packages (seed gate, private moves and repo groups, poll-claim release and feed authority, one-call `update_epic`, store import, ssh configuration, admin screen, checks for repeated findings), read enforcement (tables private, per-caller views, usage on demand, `own_subscriptions`), detached agents in the agent-tree pane, the new-task mark per task, partly hidden epics, agent build limits (`systemd-run` scope, `CARGO_BUILD_JOBS`) and Team Reviews from `teams.conf`. The next run starts at `071ba533`.

**How.** Second run of `.claude/skills/repo-health-review`. Five read-only reviewers: (1) status and regression audit of the module fix commits; (2) status and regression audit of the board, CLI and check fix commits; (3) a defect hunt over the read-enforcement commits; (4) a defect hunt over the TUI, hooks, agent-tree, build-limit and Team Reviews commits; (5) the fresh area, `src/feed/` and `src/setup/`. I re-read the code for M1, M2, M3, Y1, Y2, Y4, U2, U5, K1, G1 and the hotspot `crud.rs` diff.

**Gates (on `071ba533`).** `cargo test --no-fail-fast`: 32 targets, 6,672 passed, 0 failed, 2 ignored, no `skipping:` lines. `cargo clippy --all-targets -- -D warnings`: clean. Coverage not re-measured.

**Confidence key.** *Verified*: I re-read it in the code. *Verified by the reviewer*: a reviewer re-read it, I did not. *Plausible*: the path exists but the outcome depends on environment or timing.

## 1. Executive summary

- **The 2026-10-31 work packages landed: 29 of 36 findings Fixed**, 7 partly fixed (V1, V2, V4, V6, V8, R5, C3). Read enforcement (#98691) also landed, so the sight rules now hold on the read side too.
- **Two Highs, both in last round's fixes.** The V1 seed gate depends on a `store_owner` row that existing stores lack and that the first caller of `claim_store_owner` can take (M1). The V8 feed-authority gate covers only `patch_epic`; creating an epic, or claiming its poll, still lets one member run a shell command on another member's host (M2).
- **"A sibling path forgets the rule" recurred a third time** (M2, M4, M5, Y2, K2). Both checks that 4c85d7bd added to catch these classes have holes (G1, G2). Fixing the checks is the deliverable, not more one-off fixes.
- **Read enforcement shipped without a rules-version bump for its last step** (Y1), and the board can wrongly erase an agent or a poll claim on an update (Y2).
- **Fresh area:** feed teardown ignores which host owns a task (F1), feeds resolve repo paths against other hosts' clones (F2), and setup rewrites live hook scripts in place (F4).

## 2. Status of the 2026-10-31 findings

| # | Finding | Status | Where |
|---|---|---|---|
| V1 | Seed reducers check no caller | **Partly fixed** | fb7b2c05 gates every `seed_*` on `store_owner`; restore broken on old stores, first-come claim (M1); `burn_id_sequence`, `set_schema_version` ungated (M7) |
| V2 | Re-parenting hides others' work | **Partly fixed** | b28d2261 (`check_epic_move`, `check_task_move`); b5c9818a reopens the single-task case (M3) |
| V3 | Repo-group routing ignores visibility | Fixed | b28d2261 (`check_repo_group_stays_public`, `check_origin_patch`); repo groups made private before the fix are not migrated |
| V4 | Client-supplied `owner`/`created_by` | **Partly fixed** | fb7b2c05, d2073cbe; `TaskPatch.owner` still unstamped (M5) |
| V5 | Client-supplied `subscriber` | Fixed | fb7b2c05, d2073cbe; conformance covers only unsubscribe (M8) |
| V6 | Moves strand poll claims | **Partly fixed** | 528eb079; three other paths still strand claims (M4) |
| V7 | `archive_stale_learnings` ungated | Fixed | 528eb079 |
| V8 | Anyone changes `feed_command` | **Partly fixed** | 528eb079 gates `patch_epic` only (M2) |
| V9 | Shared PR Reviews root | Fixed | 528eb079 (by creator, private); the private flip strands claims (M4) |
| V10 | Cross-workspace moves keep stale data | Fixed | b28d2261 |
| V11 | Unlink checks no sight | Fixed | b28d2261 (`item_seen`) |
| V12 | Super-admin deletes any private item | Fixed | b28d2261 (`epic_is_unseen`, `package_is_unseen`) |
| R1 | `GIT_SSH_COMMAND` overrides `core.sshCommand` | Fixed | 7db4940b; leaves P1 back for the default setup (K3) |
| R2 | `relink_task` keeps the unseen mark | Fixed | b28d2261 |
| R3 | `update_epic` several writes | Fixed | 34aaba57; twin residue (M6) |
| R4 | Close path on every tree exit | Fixed | 747e0335 (`editor_took_over`) |
| R5 | Feed exec not non-interactive | **Partly fixed** | 7db4940b sets `GIT_TERMINAL_PROMPT=0`; BatchMode only with `GIT_SSH_COMMAND` set (K3) |
| R6 | Stale doc comments | Fixed | 765bd988, d6e86328, 714b59ab |
| R7 | N2 test copies its oracle | Fixed | 747e0335 (ICU oracle) |
| R8 | Lost live-store diagnostics | Fixed | 747e0335 |
| B1–B7 | Admin screen, membership, cursors, cache no-ops, ordering, undecodable rows | Fixed | 765bd988; B7's fix regressed (Y4) |
| C1 | Import stops an adopted store | Fixed | d6e86328 (`stop_spawned_on_exit`) |
| C2 | `set-verify` writes before checking | Fixed | d6e86328 (validates first) |
| C3 | `set-verify` takes over a host's row | **Partly fixed** | d6e86328 fixed the CLI; the module still re-stamps (K2) |
| C4–C6, C9 | Unmounted disk, blank commands, scratch store, status-line exit | Fixed | d6e86328 |
| C7 | Trajectory files grow | Fixed | 714b59ab (64 KiB cap, 90-day prune; `args` uncapped) |
| C8 | Docs and help drift | Fixed | 714b59ab |
| W2 (standing) | Unowned repo paths never pruned | Fixed, regression | 714b59ab (K1) |
| S2 residue (standing) | `close_pr_on_done` reads the cache | **Open** | `src/service/tasks/crud.rs::TaskService::update_task`; task #98747 is Done but this is not |
| S3 residue (standing) | Service epic pre-checks | Fixed | 34aaba57; `service/epics.rs::require_epic_accepting_work` now has no callers |
| T2 residue (standing) | `free_address` binds and drops | **Open** | `startup_smoke.rs`, `managed_store_real.rs`, `live_store.rs::try_start`; task #98751 is Done but this is not |
| Graduate checks | Move-path conformance, single-write check | Landed, both flawed | 4c85d7bd, a134d635 (G1, G2) |
| Spec: who the operator is | Settled | fb7b2c05, d2073cbe (`spacetime-seed.allium`: the store publisher) |
| Spec: D5 vs nearest holder | Settled on nearest holder | b28d2261; open question left at `workspaces.allium` ~1249 |
| Spec: SetEpicVisibility wording | Settled | The "creator alone" wording is gone |
| Spec: hook and ingest sight | Closed | `workspaces.allium` ~977–984 |

Yield: 29 of 36 fixed; 7 partly (V1, V2, V4, V6, V8, R5, C3). Standing items: W2 and S3 fixed, S2 and T2 still open.

## 3. Findings

Every module finding with a twin counterpart must be mirrored in `src/sync/memory_caller/` and get a case in `tests/memory_caller_conformance/`.

### 3.1 Module: authority and sight (regressions in the V-fixes)

**M1. High — fix for V1 → restore is broken on existing stores, and the publisher role goes to whoever claims it first.** `spacetime/module/src/lib.rs::claim_store_owner`, `::init`; `src/spacetime/managed_store.rs::SpacetimeCliPublisher::publish`; `src/cli/store_import.rs::import_store`. *Verified.* `init` records the publisher only on a brand-new database, so every store published before fb7b2c05 has no `store_owner` row. Nothing in `src/` or `scripts/` calls `claim_store_owner`; only its doc comment says to run it by hand. Scenario (a): `dispatch store import` into the existing managed store fails with "only the store's publisher may restore (seed) data", with no hint at the fix. Scenario (b): `claim_store_owner` checks no caller, so on a shared store the first board that calls it becomes the publisher, then calls `seed_user_profiles` to make itself super-admin and `seed_memberships` to make itself admin of every workspace. Fix: have the publish/bring-up path call `claim_store_owner` as the same CLI login straight after `spacetime publish`, treating "already has a recorded publisher" as success; check whether SpacetimeDB exposes the database owner to reducers and gate `claim_store_owner` on it (note `ctx.identity()` is the module's own identity, not the publisher's); if it does not, document the window; point the import refusal at the fix. Add a test that imports into a store published by an older module.

**M2. High — fix for V8 → any member can still make a teammate's host run a shell command.** `spacetime/module/src/tasks_epics.rs::may_change_feed_command`, `::create_epic`, `visibility.rs::settle_new_epic`, `agent_state.rs::claim_poll_owner`, `::override_poll_owner`; `src/feed/mod.rs::FeedRunner::this_host_polls`, `src/models/poll_action.rs::decide_poll_action`, `src/runtime/epics.rs::exec_trigger_epic_feed`. *Verified* (claim and authority); the run paths *Verified by the reviewer*. 528eb079 gates only `patch_epic`. Scenarios: (a) Bob creates a public root epic with `feed_command = "curl …|sh"`; every member's board that sees it finds no claim, takes `ClaimAndProceed` and runs it with that member's `gh` and ssh credentials. (b) `claim_poll_owner` and `override_poll_owner` check only `require_acting_for(host)`, not sight of `scope_id`; Bob claims Alice's epic for his host, which passes the poll-owner arm of `may_change_feed_command`, rewrites the command, and Alice's host runs it once she takes the claim back. The same call lets anyone take or stall the poll of an epic they cannot see. (c) The manual `r` refresh runs the command on any viewer's host. `FeedCommandAuthority`'s own reason ("changing it is changing what another person's machine runs") is not met. Fix, spec first (`feeds.allium`): record who last set `feed_command`; a host runs a feed (auto or manual) only when that setter is the host's owner, or after a local approval of the command's hash; apply the authority at creation; require sight of `scope_id` in claim and override; drop the poll-owner arm from `may_change_feed_command`.

**M3. Medium — decision: fix for V2 → b5c9818a lets any member hide a teammate's task again.** `spacetime/module/src/visibility.rs::task_move_permitted` (`epic_id == 0 && before.is_none()`), with d2073cbe's owner stamp in `tasks_epics.rs::move_task_row`. *Verified.* The commit says "moving a task the caller sees off a public epic onto [their board] hides nothing new". Scenario: Alice detaches Bob's running task from a shared public epic; it lands on Alice's board, `task_visible_to` now shows it to Alice alone, and Bob's board and hooks get "does not exist". This is the single-task half of V2 and the "takes it" half of V4. **The user decides:** keep the rule and correct the spec rationale, allow the detach only for the task's creator or an admin, or keep the task visible to its previous viewers.

**M4. Medium — fix for V6/V9 → three sibling paths still strand poll claims.** `spacetime/module/src/visibility.rs::set_epic_visibility_row` (public → private), `tasks_epics.rs::patch_epic_row` (re-parent under a private epic), `feed.rs::ensure_role_epic` (the V9 `make_private` flip). *Verified by the reviewer.* None calls `release_poll_claims`. Scenario: Carol's host polled the formerly shared PR Reviews root; the creator's board flips it private; Carol's `ingest_feed` gets "does not exist", no one else claims, and the feed stalls. Fix: one helper, "release claims of hosts whose owner no longer sees the epic", called from all five loss-of-sight paths; a conformance case per path.

**M5. Low — fix for V4 → `TaskPatch.owner` is still taken from the client.** `spacetime/module/src/rules.rs::check_task_patch`, `tasks_epics.rs::apply_task_patch`. *Verified by the reviewer.* The board never sends `owner` (`src/sync/encode.rs` sends `None`). Scenario: a task's owner patches `owner: "x"`; the task is orphaned for good, since `AdoptOwnerless` adopts only empty owners. Fix: refuse `owner` in `check_task_patch`.

**M6. Low — fix for R3 → the twin's `update_epic` can write part of an update.** `src/sync/memory_caller/tasks_epics.rs::apply_update_epic`. *Verified by the reviewer.* It checks every part against the row before the patch, then applies the parts and ignores their `Refused` outcomes. Scenario: a root epic gets `{parent_epic_id: P, workspace_id: W2}`; the module patches, `move_epic_row` refuses "is a sub-epic" and the call rolls back; the twin writes the parent and answers `Applied`. `{parent: 0, workspace: W2}` diverges the other way. Fix: propagate the apply-phase refusal (or apply in the module's order on a copy); add a conformance case.

**M7. Low — sibling restore reducers are ungated.** `spacetime/module/src/sequence.rs::burn_id_sequence`, `::probe_generated_task_id`, `lib.rs::set_schema_version`. *Verified by the reviewer.* Any board can burn an id sequence (a large `ceiling` stalls the transaction) or rewrite `schema_version`. Fix: `require_restore_authority` on each, with a refusal test.

**M8. Low — conformance gaps in last round's fixes.** fb7b2c05 mirrored its changes in the twin but added no case for the `subscribe_to_epic`/`mark_epic_seen` refusals or the epic and package `created_by` stamps; `private_moves.rs` does not cover `ReParentingKeepsTheHolder`; ab512934's `r.epic(task.epic_id).is_some()` guard cannot change an outcome (`visible_task` already hides a task with a missing epic) and has no test. *Verified by the reviewer.* Fix: add the cases; drop or test the guard.

### 3.2 Sync: read enforcement and the board's rows

**Y1. Medium — 2e681e34 made `subscriptions` private without a rules-version bump.** `spacetime/module/src/rules.rs::RULES_VERSION`, `src/sync/rules_gate.rs::BOARD_RULES_VERSION`, both 31 since 5edab153. *Verified.* A board built between 5edab153 and 2e681e34 passes the gate against the new module, sends `SELECT * FROM subscriptions WHERE subscriber=…`, which a private table refuses; the whole subscribe fails, `SyncSession::attempt` drops and retries forever with no "upgrade your board" message. A new board against a 31 module asks for an `own_subscriptions` view that does not exist. Medium, not High, because the two commits were hours apart. Fix: bump both to 32, and add to the `RULES_VERSION` doc that a change to the board's subscription set warrants a bump (87a38ea3 set the precedent).

**Y2. Medium — fix for the view false-erase (904b8be7) → agents and poll claims still suffer it.** `src/sync/rows.rs::SharedRows::retire_agent`, `::retire_poll_owner`; `src/sync/decode.rs::agent` (drops `stop_pending_at`), `::poll_owner` (drops `claimed_at`). *Verified.* Both compare decoded rows. Scenario: `agent_state.rs::try_record_stop` with live subagents rewrites only `stop_pending_at`; the old and new rows decode equal, so the delete half of the update removes the agent from the board and `current_agent(task)` answers `None` until the next write. A forced same-host `write_poll_owner_row` does the same to a claim. `sync.allium: AViewUpdateNeverErasesTheRow` covers every table behind a view. Fix: keep and compare the raw row as 904b8be7 did for tasks and epics; add tests like `view_updates.rs::a_task_update_in_an_ignored_column_*`.

**Y3. Medium (plausible) — requests and agent rows vanish when a task stops being held.** `spacetime/module/src/views.rs::dispatch_requests_of_held`, `::session_requests_of_held`, `::agents_of_held`; `src/sync/sdk_connector/queries.rs::subscription_queries`. *Verified by the reviewer.* Before 87a38ea3 requests were served `WHERE host = …`; now only for held tasks (own tasks plus followed epics). Scenario: I dispatch a teammate's task from an epic I follow, then unfollow it. `request_wrap_up` writes its row, but the read-back in `callers.rs` never finds it and the agent gets an error; my reactor never serves it, so the window is never torn down. Fix: serve request rows by `host` among the hosts the caller owns (`hosts_owned_by`); state the rule in `sync.allium`.

**Y4. Low — fix for B7 → an undecodable-to-undecodable update erases the mark.** `src/sync/rows.rs::SharedRows::retire_task` (765bd988), `::upsert_epic`'s error branch. *Verified* (`retire_task`). `retire_task` removes `undecodable_tasks[id]` whenever the old row fails to decode; an update arrives as insert(new) then delete(old), so a second undecodable version loses the mark `DeleteEpicRefused`'s pre-check counts. `upsert_epic` keeps the old decoded epic on a decode error, which paired views can leave on the board. Fix: remove the mark only when it still belongs to the departing row (compare raw); in `upsert_epic`'s error branch remove the epic, as `upsert_task` does.

**Y5. Low — the twin does not apply the views to the board's rows.** `src/sync/memory_caller/workspaces.rs` (`remove_member` only calls `rows.remove_membership`), `tasks_epics.rs` (unconditional `upsert_task`/`upsert_epic`). *Verified by the reviewer.* A test over the twin still shows a removed member's tasks, a private-flipped epic or an unfollowed subtree, so board-side loss-of-sight bugs like Y3 cannot fail there. Fix: recompute the twin's board rows from the `own_*`/`covered_*`/held view functions after each apply, or record the gap in `docs/testing.md`.

### 3.3 TUI, hooks, agent tree and agent launch

**U1. Medium (plausible) — two resumes can start two `claude --continue` in one worktree.** `src/tui/update/agent.rs::handle_resume_task`, `src/dispatch/agents.rs::resume_agent_with_message`. *Verified by the reviewer.* The only guard is `task.tmux_window.is_some()`, set when `Resumed` returns; `tmux::has_window` runs before `new_window` in separate `spawn_blocking` calls. b306f148 adds Space on a detached row in the pane, with no "resuming" state, so a second press is likely; a pane resume plus MCP `resume_task` also overlap. Fix: an in-flight resume set on `App`, refused while held, cleared on `Resumed` or error.

**U2. Medium — upgrading empties Team Reviews, and a failed search deletes tasks.** `scripts/fetch-reviews.sh::search_prs`, `src/setup/plugins.rs::SHIPPED_SCRIPT_CONFIGS` (071ba533). *Verified.* The script auto-updates, `teams.conf` ships `TEAMS=()`, and the repo-scoped `review-requested:@me` search is gone, so the first cycle after upgrading emits no `team-request` items and reconcile deletes every Team Reviews task. Nothing tells the user to edit `teams.conf`. Separately, `search_prs` prints `[]` and returns 0 on a failed `gh search` (pre-existing), and each team adds a search against the 30/min pool, so a rate-limited cycle deletes and later re-creates tasks, re-firing their new-task marks. Fix: show a hint when `TEAMS` is empty; make a failed search fail the emission (non-zero exit) so the degraded guards stop the reconcile.

**U3. Medium — `CARGO_BUILD_JOBS=8` overrides a repo's own `build.jobs`.** `src/dispatch/mod.rs::agent_launch_env` (f0331faf). *Verified by the reviewer.* Cargo ranks the environment above `.cargo/config.toml`, so a repo or user that set `jobs = 2` to save memory gets 8; under 8 cores it oversubscribes. Fix: `min(8, nproc)`, skip when `build.jobs` is configured, and say so in `AgentBuildsRunFewerParallelJobs`.

**U4. Medium (plausible) — the scope check and the scope launch see different environments.** `src/process/agent_scope.rs::prepare_agent_scope`, `AgentScope::wrap`. *Verified by the reviewer.* The check runs `systemctl --user` from the board; the wrapped command runs in the tmux pane's shell with the tmux server's `XDG_RUNTIME_DIR`/`DBUS_SESSION_BUS_ADDRESS`, and `systemd-run` on `PATH` is never checked. A tmux server started from ssh or a TTY gives "Failed to connect to bus": the window holds no agent and the board does not say so, which the spec's best-effort clause forbids. Fix: probe `systemd-run --user --scope -- true` once per board (cached), and fall back: `systemd-run … || exec claude …`.

**U5. Medium on macOS — the peer-message hook aborts on bash 3.2.** `plugin/hooks/scripts/task-status-hook` (63b8488a). *Verified.* `"${MSG_FLAGS[@]}"` with `MSG_FLAGS=()` under `set -u` is an "unbound variable" error on bash ≤ 4.3; `aarch64-apple-darwin` ships and macOS `/bin/bash` is 3.2, so every delivered `SendMessage` observation exits 1. Fix: `${MSG_FLAGS[@]+"${MSG_FLAGS[@]}"}`; consider a shellcheck or bash-3.2 lint for shipped hook scripts.

**U6. Low — Space cannot remove a hide on a hidden-by-parent row.** `src/tui/update/board_filter.rs::handle_toggle_hidden_epic` (7f0227b4). *Verified by the reviewer.* The code branches on the row mark, and `[-]` wins over the row's own hide, so Space always inserts; `ToggleHiddenEpic` says it removes when `epic in hidden_epics`. Fix: branch on `hidden.contains(&id)` first.

**U7. Low — a resumed agent's wait never times out.** `src/agent_tree/state.rs::adopt_agent_list`. *Verified by the reviewer.* `Accepted` means only "handed to the board"; a failed launch leaves `pending_jump` set for ever with no notice in the pane. Fix: a deadline, then a pane notice.

**U8. Low — scope and sccache start sit outside the dispatch deadline.** `src/dispatch/request_host.rs::DeadlineRunner` delegates `agent_scope` and `start_sccache_server`; two 10 s `systemctl` calls and an untimed `sccache --start-server` run on every launch (and warn on macOS). *Verified by the reviewer.* Fix: cache the scope decision per board, time out the sccache start.

### 3.4 Fresh area: `src/feed/` and `src/setup/`

**F1. Medium — feed teardown ignores which host owns the task.** `src/feed/mod.rs::cleanup_removed_feed_tasks`, `src/store/queries/mod.rs::confirm_removed`. *Verified by the reviewer.* `RemovedFeedTask` has no `host`; host B (poll owner, or whoever pressed `r`) runs `teardown_task` for a task dispatched on host A, kills nothing of its own, and A's window (with a live agent) and worktree outlive the deleted row. This bypasses the session seam. Fix: carry `host`; tear down locally only when it is this host, otherwise write a teardown request for the owner's reactor.

**F2. Medium — feeds resolve repo paths against every host's clones.** `src/feed/cycle.rs::FeedCycle::sync_emission`, `src/feed/mod.rs::FeedRunner::tick`, `src/dispatch/mod.rs::resolve_repo_path`. *Verified by the reviewer.* The cycle uses `list_repo_paths()` (all hosts) where `IngestFeedEmission` says local clones and `list_repo_paths_saved_by_this_host` exists. A teammate's `/home/alice/Code/api` gets stamped on the task; dispatch from this host targets a missing path. Short-name matching also maps `orgA/api` and `orgB/api` to one clone. Fix: use this host's list and match on the clone's `owner/repo`.

**F3. Medium — feed cycles are serialised per process; the spec promises per epic.** `src/feed/guard.rs::FeedSyncGuard`; `feeds.allium: SerialisedFeedCycle`, `ManualFeedTrigger` ("on ANY host"). *Verified by the reviewer.* Manual refresh runs on non-owner hosts and `ClaimAndProceed` proceeds before its claim lands, so two hosts can reconcile the same epic; an older emission from B can delete what A just inserted. Fix, spec first: make a manual refresh on a non-owner host request a cycle from the owner (or run additive-only), or state that the guard is per host.

**F4. Medium — setup rewrites live hook scripts and settings in place.** `src/setup/mod.rs::write_file_if_changed`, `plugins.rs::write_shipped_script`, `::write_script_manifest`. *Verified by the reviewer.* `fs::write` (truncate then write) for every plugin file including `hooks/scripts/*.sh`, `dispatch-statusline.json` (read at every spawn) and the feed scripts; for a new hook, `set_permissions(0755)` comes after. A board restart runs this while agent windows live; bash reads scripts as it goes, so a hook can run a torn file. Fix: `crate::atomic_write::write_bytes` with the mode set at creation.

**F5. Medium (plausible) — updates to `~/.claude.json` can be lost.** `src/setup/config_update.rs::apply_mcp_setup`, `config.rs::remove_mcp_config`. The read-merge-rename is atomic against torn files but not against Claude Code's own frequent writes; a trust entry written between our read and rename is lost. Fix: re-read and compare (mtime or hash) just before the rename and retry.

**F6. Low — a failing feed with empty stderr reports nothing.** `src/feed/exec.rs::exec_feed_command` returns `Err(stderr)`; `exit 1` or a quiet `jq` shows "Failed: " with no exit code. *Verified by the reviewer.* Fix: include `output.status`.

**F7. Low — a below-floor interval logs every 2 seconds.** `src/feed/mod.rs::epic_due` warns before bumping `last_run`: ~43k lines a day per epic in an unrotated `app.log`. The spec says "EVERY tick". *Verified by the reviewer.* Fix, spec first: warn once per epic per value.

### 3.5 CLI, process and repo paths

**K1. Medium — fix for W2 → prune deletes another host's legacy row.** `src/cli/commands.rs::prune_repo_paths` (the unowned loop); `cli.allium: PruneRepoPathsViaCli`. *Verified.* An unowned row missing on *this* disk is deleted. Scenario: host B prunes first and deletes Alice's legacy `/home/alice/proj` row, verify command included, before host A can adopt it. The same guidance says a path "says nothing about this disk". Fix, spec first: adopt when present, never delete an unowned row.

**K2. Medium — fix for C3 is client-side only; the module still re-stamps the host.** `spacetime/module/src/config.rs::save_repo_path`, `::delete_repo_path`; `src/runtime/tasks.rs::exec_save_repo_path`. *Verified by the reviewer.* Every TUI save of an existing path makes the caller's host its owner, and K1's adoption relies on that re-stamp, so the two pull in opposite directions. `delete_repo_path` deletes by path with no host guard, so a prune from a stale cache deletes a row another host just adopted. Fix: an `adopt_repo_path` reducer that stamps only when `host == ""`; `save_repo_path` never changes a non-empty host; `delete_repo_path` takes the expected host.

**K3. Low — fix for R1 → P1 is back for the default ssh setup, and the feed still lacks it (R5).** `src/process.rs::ssh_config_args`, `::configured_ssh_command`; `src/feed/exec.rs::run_feed_command_bounded`. *Verified by the reviewer.* With none of `GIT_SSH_COMMAND`/`core.sshCommand` set, git runs ssh without BatchMode; a passphrase key hits SIGTTIN and reads as a timeout. Each bounded git call also spawns a `git config --get` with its own 10 s timeout outside the caller's deadline. Fix: start bounded children with `setsid` (own group, no controlling tty, so ssh fails at once and no config is touched); this also covers the feed. Subtract or cache the lookup.

**K4. Low — a refused workspace create still moves the admin cursor.** `src/tui/update/admin.rs::select_workspace_named`. *Verified by the reviewer.* A duplicate-name create is refused but the next snapshot highlights the existing workspace of that name. Fix: clear the pending name on refusal.

### 3.6 The graduated checks

**G1. Medium — the move-path guard checks the opposite of the bug class.** `tests/memory_caller_conformance/move_paths.rs::every_task_move_site_in_the_module_is_in_the_table`. *Verified.* It collects functions that *call* `clear_unseen_for_task(` and requires each in the table; N1 and R2 were move paths that *forgot* the call, so R2's `relink_task` would have passed. The sight rule is exercised only with a missing epic, never another user's private one. Fix: find the sites that write a task's `epic_id` and require each in the table; add a private, unseen destination case.

**G2. Medium — `check-single-write-service.sh` misses wrapped calls and free functions.** `scripts/check-single-write-service.sh`. *Verified by the reviewer.* The awk matches `self.db.<m>(` on one line; rustfmt wraps many as `self\n.db\n.m(` (`set_member`'s two `add_member` calls, `update_task`, `set_task_epic_id`, `patch_task` …). It resets on any column-0 `fn` and counts only `self.db.`, so `src/service/managed_feeds.rs::write_managed_feed_settings` (four `db.set_*` writes) is invisible; writes in a loop count once; an allow-listed function can gain writes. Fix: normalise `self\s*\.\s*db\s*\.` across lines (or use `syn`), count any `db.` receiver in every `fn`, record the expected count in the allow-list, add `session_request` to READS; then make `write_managed_feed_settings` one reducer or allow-list it with a reason.

### 3.7 Spec disagreements to settle (not code fixes)

- `feeds.allium: FeedCommandAuthority`'s rationale vs its grant to the poll owner and the auto-claim (M2).
- `feeds.allium: SerialisedFeedCycle` global vs per-process guard (F3); the same rule still names `kill_on_drop`, the code kills the process group.
- `feeds.allium` "warn EVERY tick" for a below-floor interval (F7).
- `ToggleHiddenEpic` vs 7f0227b4 (U6).
- `sync.allium: SubscribeOnceIdentityIsSettled` guidance (~546–553) still says usage comes through a view (5edab153 removed it) and that subscriptions are public with "known gap #98806" (2e681e34 fixed it); `src/sync/rows.rs::Rows` doc comments say the same.
- `workspaces.allium` ~1247 asks the question b5c9818a answered; ~1251 says packages carry no `created_by` (fb7b2c05 stamps them); ~986 "an epic with no creator is stamped" (all are now); ~1245 says moves onto a user board drop links (they keep them).
- `b5c9818a`'s "hides nothing new" vs `WritesRequireSight` (M3).

### 3.8 Also noted (not filed)

- The pane lists other machines' detached agents; Space on them is always refused. Spec-conformant (`DetachedIsTheBoardsOwnDefinition`).
- `run_feed_command_bounded` kills the group only on timeout; background children that keep stdout open turn a valid emission into a timeout (plausible).
- `live_store.rs::try_start`'s `is_address_in_use` reads stderr only (plausible).
- `--body "$MSG_BODY"` in the hook: a body starting with `-` may parse as a flag (pre-existing, plausible).
- `service/epics.rs::require_epic_accepting_work` is `pub`, re-exported and has no callers.
- Repo groups made private before b28d2261 are not migrated.
- Since 87a38ea3, `SpacetimeCliStore::rows` relies on the CLI login owning the database; no test covers a non-owner dump (plausible).
- An already-connected old board may keep a subscription to a table a hot-swap made private until it reconnects; worth one live test (plausible).

## 4. Hotspots and agent context

**Hotspots** (`git log --since=60.days -- src spacetime/module/src`; change count, current lines).

| File | Changes | Lines |
|---|---|---|
| src/spacetime/module.wasm.source-hash | 93 | 1 |
| src/spacetime/module.wasm | 85 | binary |
| src/runtime/mod.rs | 71 | 1,258 |
| src/service/tasks/crud.rs | 55 | 956 |
| spacetime/module/src/lib.rs | 55 | 328 |
| src/spacetime/bindings/mod.rs | 53 | 3,046 (generated) |
| src/tui/mod.rs | 52 | 927 |
| src/sync/writes.rs | 51 | 737 |
| src/store/mod.rs | 51 | 1,340 |
| src/runtime/tasks.rs | 48 | 1,263 |
| src/tui/types.rs | 44 | 579 |
| src/mcp/handlers/tests/mod.rs | 43 | 1,699 |
| src/mcp/handlers/dispatch.rs | 43 | 1,058 |
| src/db/mod.rs | 43 | removed |
| src/models/tasks.rs | 41 | 1,178 |
| src/sync/tests/writes.rs | 40 | removed |
| src/runtime/tests/mod.rs | 40 | 972 |
| src/sync/rows.rs | 39 | 1,303 |
| src/runtime/tests/misc.rs | 38 | 1,280 |
| src/main.rs | 37 | 1,061 |

Changes since 2026-10-31: `src/sync/rows.rs` (396 lines changed in range; Y2, Y4) and `src/main.rs` entered; `src/dispatch/tests.rs` and `src/tmux.rs` left. `crud.rs` rose from 50 to 55: the in-range diff removes the service's epic pre-checks (S3) and maps the module's refusals to `NotFound` by matching the refusal text `"epic {id} does not exist"`, which couples the service to the module's wording. `src/runtime/mod.rs` was untouched in range and `src/models/tasks.rs` changed one line. The `src/store/mod.rs` diff (87 lines) is trait reshaping for the one-call `update_epic`/`update_work_package` (`WorkPackageFields`), `list_agent_tasks` (live and detached) and `mark_task_seen`; no finding. In the module, in-range churn is spread over `rules.rs` (8), `visibility.rs` (7), `config.rs` (5), `tasks_epics.rs` (4).

**Agent context** (`wc -w`; 2026-10-31 → now): `CLAUDE.md` 2,281 → 2,281; `docs/invariants.md` 1,484 → 1,486; `docs/conventions.md` 13,029 → 13,096; `docs/reference.md` 9,675 → 9,874; `docs/testing.md` 4,424 → 4,501; `docs/module-map.md` 4,939 → 4,938; `docs/architecture.md` 2,035; `docs/how-to.md` 1,784; `docs/mcp.md` 2,836; `docs/mcp-tool-design.md` 875. All `CLAUDE.md` + `docs/*.md`: 43,359 → 43,706 (+0.8%). Flat; no action.

`docs/invariants.md` / `CLAUDE.md` seams checked against code: `TaskService::request_dispatch`, `::serve_dispatch_request` (only `src/runtime/dispatch_host.rs` calls it), `::report_pr_status`, `::report_pr_unreadable`, `::request_wrap_up`, `::request_exit_session`, `::serve_session_request` (only `src/runtime/session_host.rs`) all exist where cited; `RULES_VERSION` and `BOARD_RULES_VERSION` are tied by a test in `src/sync/rules_gate.rs`. All hold.

## 5. Standing items

- **S2 residue** — `TaskService::update_task` reads `prior` from the cache for `close_pr_on_done`. Raised 10-22, 10-28, 10-31 and now; task #98747 was closed Done without it. **Ask the user:** file again as its own package, or accept it.
- **T2 residue** — `free_address` binds and drops a port in three test harnesses. Raised 10-28, 10-31 and now; task #98751 was closed Done without it. **Ask the user** the same.
- **M3** — the detach rule is a design decision; see the finding.

Filed: the user chose one package for S2 and T2, task #98860. All findings are in epic #69683 (tasks #98850–#98861, plans in `docs/plans/review-2026-11-03/`).

**Graduate to a check.** The class "a sibling path forgets the rule its fix added" appeared in N1 (10-28), R2 (10-31) and now M2, M4, M5, Y2 and K2. Last round's checks were meant to catch it and do not (G1, G2). The deliverable is G1 and G2 fixed, plus one new conformance test that runs every loss-of-sight path (visibility flip, re-parent, workspace move, member removal, managed-root flip) against a "poll claims released" assertion (M4), and a `sync` test that, for every table behind a view, sends an update that changes only a column the decoder drops (Y2).

## 6. Areas read

- Every diff in `e68f86a1..071ba533`.
- In full or near full: `spacetime/module/src/{lib,seed,visibility,tasks_epics,agent_state,feed,config,views,rules,sequence}.rs` and the matching twins in `src/sync/memory_caller/`; `src/sync/rows.rs`, `src/sync/decode.rs`, `src/sync/sdk_connector/queries.rs`; `src/process.rs`, `src/process/agent_scope.rs`; `src/cli/commands.rs`; `scripts/check-single-write-service.sh`, `scripts/fetch-reviews.sh`; `tests/memory_caller_conformance/move_paths.rs`; `plugin/hooks/scripts/task-status-hook`.
- Fresh area: `src/feed/` (`mod.rs`, `cycle.rs`, `exec.rs`, `guard.rs`, `parse.rs`, `ingest/mod.rs`) and `src/setup/` (`mod.rs`, `config.rs`, `config_update.rs`, `confirm.rs`, `hooks.rs`, `plugins.rs`, `statusline.rs`, `uninstall.rs`); oracles `feeds.allium`, `tasks.allium`.
