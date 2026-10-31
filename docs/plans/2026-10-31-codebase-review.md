# Codebase review — 2026-10-31

Task #98742. Follows `docs/plans/2026-10-28-codebase-review.md` (task #98508). This report replaces an earlier 2026-10-31 report by task #98654 (still in git at 80ead837); the operator removed that report's tasks and asked for a fresh, independent review. It was written without reading the earlier one.

**Range.** `00ef35ab..e68f86a1` on `main`, 42 commits (no merges). Main themes: the seven 2026-10-28 work packages, the removal of repo sync, the workspaces feature (columns and migration, sight checks on writes, per-caller views, admin screen and MCP tools, workspace-scoped learnings, moving a root epic between workspaces), the `not_applicable` learning verdict, and the My Reviews signal fix. The next run starts at `e68f86a1`.

**How.** First run of `.claude/skills/repo-health-review`. Four read-only reviewers: (1) status of the 2026-10-28 findings and a regression audit of the 18 fix commits; (2) a defect hunt over the workspaces module and twin; (3) a defect hunt over the board, sync and MCP side; (4) the fresh area, `src/cli/` and `src/mcp/trajectory.rs`. I re-read the code for V1, V2, V6, V9, R1, B1, B2/R3 and C1.

**Gates (on `e68f86a1`).** `cargo test --no-fail-fast`: 32 targets, 6,597 passed, 0 failed, 2 ignored, no `skipping:` lines. `cargo clippy --all-targets -- -D warnings`: clean. Coverage not re-measured (last figure 92.17%, 2026-10-15).

**Confidence key.** *Verified*: re-read in the code. *Plausible*: the path exists but the outcome depends on environment or timing.

## 1. Executive summary

- **The 2026-10-28 work packages landed well: 17 of 23 findings Fixed**, 4 partly fixed, 1 moot, W2 still open. `PaneId` and the one-transaction `update_task` are done and mirrored in the twin.
- **One High that undoes the workspace model:** no `seed_*` reducer checks its caller, so any connected board can make itself super-admin or rewrite any epic (V1).
- **Moves are the weak point of the sight rules.** Re-parenting under your own private epic, repo-group routing and flatten, and client-chosen `owner`/`created_by`/`subscriber` all let one member hide or take another's work (V2–V5).
- **Two regressions in last round's fixes:** the P1 fix overrides the user's `core.sshCommand` (R1), and `update_epic` repeats the S1 mistake as three store writes (R3).
- **Fresh area:** `dispatch store import` stops a running board's store when it adopted it instead of starting it (C1).

## 2. Status of the 2026-10-28 findings

| # | Finding | Status | Where |
|---|---|---|---|
| S1 | Relink and patch were two writes | **Fixed** | 5a5209b7; module `tasks_epics.rs::update_task`, twin `apply_update_task`, conformance case |
| S2 | Done decided from the cache | **Partly fixed** | 5a5209b7, f54c9ecb; module owns the stamp. `crud.rs::TaskService::update_task` still reads `prior` from the cache for `close_pr_on_done` |
| S3 | Service re-checks existence | **Partly fixed** | 5a5209b7; `move_task_to_epic` and `create_task_inner` keep `require_epic_accepting_work` |
| N1 | Feed move keeps the unseen mark | **Fixed (feed path)** | 2e523c8d; sibling gap R2 |
| N2 | Format characters in names | **Fixed** | 2e523c8d, `user_profile.rs::FORMAT_RANGES`; weak test R7 |
| N3 | `resume_task` answers success wrongly | **Fixed** | 31599044, 54faffef |
| G1 | `v` runs the close path | **Fixed for `v`** | a29c0c51; residue R4 |
| G2–G6 | Agent-tree pane lifecycle, comments | **Fixed** | a29c0c51 |
| P1 | Prompting remote hangs | **Fixed, regression** | 8aae4b80; R1, R5 |
| P2 | Fast-forward vs repo sync race | **Moot** | a2f11560 removed repo sync |
| P3–P5 | Editor trim, header escape, CLI wording | **Fixed** | 8aae4b80, 42f060d9 |
| T1 | First board outlives drop | **Fixed** | 1c0bf2f4 |
| T2 | Port race | **Partly fixed** | 1c0bf2f4; `startup_smoke.rs::free_address` still binds and drops; R8 |
| T3 | Previous-store gate in CI | **Fixed** | 1c0bf2f4 |
| W2 | Unowned repo paths never pruned | **Open** | `src/sync/rows.rs::repo_paths_saved_by`; standing item |
| — | `PaneId` newtype | **Fixed** | 0aed824c, 7de7c5db |

Yield: 17 of 23 fixed (S2, S3, T2 and the G1 residue partly; P2 moot; W2 open).

## 3. Findings

### 3.1 Workspaces: authority and sight in the module

Every module/twin pair below has the same gap in `src/sync/memory_caller/`; fixes must mirror it and add a case in `tests/memory_caller_conformance/`.

**V1. High — the seed reducers check no caller.** `spacetime/module/src/seed.rs` (`seed_user_profiles`, `seed_memberships`, `seed_workspaces`, `seed_epics`, `seed_work_packages`, `seed_learnings`, `seed_tasks`, and the rest). *Verified*: no call to `is_operator`, `caller_identity` or a super-admin check in the file. Scenario: any board calls `seed_user_profiles([{identity: me, is_super_admin: true}])` and is super-admin; `seed_memberships` makes it admin of every workspace; `seed_epics` flips another person's private epic to public or rewrites `created_by`; `seed_learnings` picks `author` and `workspace_id`. The specs disagree on who may seed: `spacetime-seed.allium` has `actor Operator { identified_by: core/Host where id != "" }` (any host), `workspaces.allium` names the publisher identity as the trust boundary. Fix, spec first: decide which identity a restore runs as and how the module recognises it (the restore path calls these through `spacetime call` as the CLI's logged-in identity, `src/spacetime/cli_store.rs`), then gate every `seed_*` on it and add one refusal test per reducer.

**V2. High — re-parenting under your own private epic hides other people's work.** `spacetime/module/src/visibility.rs::check_epic_move` (from `tasks_epics.rs::patch_epic`) and `tasks_epics.rs::move_task_row` via `check_task_may_enter_epic`. *Verified.* Only "destination visible, same workspace" is checked. Scenario: Alice sees Bob's public epic B with running tasks; she patches `B.parent_epic_id = Q`, her private epic. B's subtree now has Alice as private holder and vanishes for Bob and his hosts (their hooks get "does not exist"). B is still `public`, so `unseen_items` never lists it and the super-admin cannot recover it. The same works for a single task. This gets around `SetEpicVisibility`'s creator-or-admin rule. Fix, spec first (`EpicStaysInItsWorkspace`, `TaskEntersAnEpicTheCallerCanSee`): refuse a move that changes an item's effective private holder unless the caller may already set its visibility.

**V3. Medium — repo-group routing and flatten ignore visibility.** `spacetime/module/src/grouping.rs::find_or_create_repo_group`, `::route_to_repo_group`, `::flatten_epic`, `::regroup_epic`; `visibility.rs::set_epic_visibility`. *Verified.* A repo-group sub-epic is found by title and origin only, and its creator (whoever routed the first task) may make it private. Scenarios: (a) Carol's `create_task` on the visible root is routed into Alice's private sub-epic and disappears from Carol's view while the call succeeds; (b) Bob's `flatten_epic(root)` moves Alice's private tasks onto the public root and deletes her sub-epic; (c) `regroup_epic` moves everyone's tasks into it. Also `patch_epic` may change `origin`, which gets around the repo-group re-parent refusal. Fix: refuse `set_epic_visibility` and `origin` changes on `origin == "repo-group"` epics (simplest), or skip unseen sub-epics in routing and refuse flatten over them. Record the rule in `epics.allium`.

**V4. Medium — the module trusts a client-supplied `owner` and `created_by`.** `tasks_epics.rs::move_task_row`, `::set_task_epic`, `::update_task` and `::create_task` take `owner` from the caller when `epic_id == 0`; `visibility.rs::settle_new_epic` keeps a client `created_by`; `check_new_package` never stamps it. *Verified.* The spec says an epic-less task "lands on the caller's own user board". Scenario: Bob detaches a teammate's task with `owner = Bob` (takes it) or `owner = "x"` (no one ever sees it again; `AdoptOwnerless` adopts only empty owners). A forged `created_by` gives another person visibility rights and fake authorship. Fix: for non-operators, stamp `owner = caller` on a detach or epic-less create and always stamp `created_by = caller`; update the `WritesRequireSight` guidance.

**V5. Medium — follow and seen reducers trust a client-supplied `subscriber`.** `spacetime/module/src/config.rs::subscribe_to_epic`, `::unsubscribe_from_epic` (no sight check at all), `unseen.rs::mark_epic_seen`. *Verified.* Scenario: Alice unsubscribes Bob from an epic, which also releases his hosts' poll claims; or clears Bob's unseen marks. Fix: derive the subscriber from `caller_identity` (operator exempt); this changes reducer signatures, so bump `RULES_VERSION` and `BOARD_RULES_VERSION` together.

**V6. Medium — moving an epic or removing a member strands feed poll claims.** `visibility.rs::move_epic_to_workspace` and `workspace.rs::drop_follows_in_workspace` (from `remove_member`). *Verified* (neither file touches `poll_owners`; `config.rs::unsubscribe_from_epic` does). Scenario: the host that polls a feed epic belongs to someone who is not in the destination workspace, or was removed. That host now gets "does not exist" from `ingest_feed`; no other host claims the epic, because a claim only fills an absent row. The feed stalls until someone runs `OverrideFeedOwner`. Fix, spec first (`MoveEpicToWorkspace`, `RemoveMember`, `sync.allium: UnsubscribeFromEpic`): release poll claims held by hosts whose owner loses sight.

**V7. Medium — `archive_stale_learnings` has no gate and no sight check.** `spacetime/module/src/learning.rs::archive_stale_learnings`. *Verified.* Any client passes `cutoff = "9999"` and archives every approved learning with no upvotes, in every workspace. A normal board's own cutoff also archives other workspaces' learnings. Breaks `WritesRequireSight`. Fix: filter by `learning_scope_seen` for the caller, or make it operator-only.

**V8. Medium — any member can change the shell command another person's host runs.** `tasks_epics.rs::patch_epic` (the `feed_command` field) and `feed.rs::ensure_role_epic`. *Verified* (pre-existing, but it now crosses the workspace trust boundary). The epic's poll-owner host runs `feed_command` (`src/feed/mod.rs`); anyone who can see the epic can set it. Fix, spec first (`feeds.allium`): only the epic's creator, the poll owner's owner or a workspace admin may change `feed_command`.

**V9. Medium (plausible) — one shared PR Reviews tree per workspace, now signalled to everyone.** `spacetime/module/src/feed.rs::ensure_role_epic` finds the managed root by `(feed_role, parent 0, visible)`, not by creator; since 300ce9cb `unseen.rs::mark_task_unseen_for_viewers` marks every viewer. Scenario: after the migration everyone is in Default, so each board reuses the first person's "PR Reviews" root and rewrites its `feed_command` to its own; whichever host polls fills "My Reviews" with its user's PRs, and every member is signalled. The spec covers only the cross-workspace case. Fix, spec first: key managed roots by creator, or make them private to their creator.

**V10. Low — cross-workspace task moves and learning rescope keep stale workspace data.** `tasks_epics.rs::move_task_row` (a task moved between epics in two workspaces keeps its package links and task-scoped learnings' `workspace_id`); `learning.rs::rescope_epic_learnings`, `::rescope_learnings` move `scope_ref` only. *Verified.* Narrower sight, no leak; breaks `LinkedItemsShareThePackagesWorkspace`. Fix: reuse `move_epic_to_workspace`'s link and learning cleanup, or refuse cross-workspace task moves.

**V11. Low — unlinking checks no sight on the item.** `spacetime/module/src/packages.rs::unlink_work_package_item` and the unlink list of `update_work_package_with_links`. *Verified.* A viewer of a public package can unlink a teammate's main-board task they cannot see. Fix: treat an unseen item as not linked.

**V12. Low — the super-admin hatch deletes any private item, not only unseen ones.** `SuperAdminDeletesUnseenItem` requires only `item_is_private`; the code matches. *Verified.* A super-admin can delete an active member's private epic. Fix, spec first: also require that the creator is no longer a member.

**Spec disagreements to settle (not code fixes):**
- Who the operator is (V1): `spacetime-seed.allium` vs `workspaces.allium`.
- Private subtrees: D5 "a private epic hides its whole subtree" vs `core.allium` `private_holder` (nearest private ancestor). Under the nearest-holder rule, Bob's private Q under Alice's newly private R is visible to Bob under an epic he cannot see.
- `SetEpicVisibility` guidance ("hands it to its creator alone") vs `WritesRequireSight` ("teammates may still act on each other's work").
- `workspaces.allium` open question (~line 988) says hook reducers and `ingest_feed` have no sight check; the code has them. Close the question.

### 3.2 Regressions in last round's fixes

**R1. Medium — fix for P1 → the user's ssh configuration is ignored.** `src/process.rs::run_bounded`, `::git_ssh_command`. *Verified.* Every bounded child gets `GIT_SSH_COMMAND` (plain `ssh -o BatchMode=yes` when unset). Git gives `GIT_SSH_COMMAND` priority over `core.sshCommand` and `GIT_SSH`. Scenario: a user with `core.sshCommand "ssh -i ~/.ssh/id_work -o IdentitiesOnly=yes"` (the usual two-account GitHub setup) now fails the wrap-up `pull` (`src/dispatch/finish.rs`) and the provisioning `fetch` (`src/dispatch/worktree.rs`) with "Permission denied (publickey)". Fix: set `GIT_SSH_COMMAND` only when the caller already set it (append BatchMode); otherwise rely on `GIT_TERMINAL_PROMPT=0` plus `-c core.sshCommand="<resolved> -o BatchMode=yes"`, or `SSH_ASKPASS_REQUIRE=never` with a null stdin. Add a test with `core.sshCommand` set.

**R2. Low — fix for N1 → the same unseen-mark gap in the sibling move paths.** `spacetime/module/src/grouping.rs::relink_task`, twin `src/sync/memory_caller/grouping.rs::relink_task`. *Verified by the reviewer.* `relink_task` changes `epic_id` without `clear_unseen_for_task`; it backs repo-change reroutes, `regroup_epic` and `flatten_epic`. Breaks `UnseenMarkEndsWithTheTaskLeavingItsEpicOrFinishing`. Fix: clear inside `relink_task` in both, plus a conformance case.

**R3. Medium — the S1 mistake repeated on the epic path: `update_epic` and `update_work_package` are several writes.** `src/service/epics.rs::EpicService::update_epic` (`patch_epic`, then `move_epic_to_workspace`, then `set_epic_visibility`); `src/service/work_packages.rs::WorkPackageService::update_work_package` (two calls). *Verified.* Scenario: a member who is not the creator sends `update_epic {title, visibility}`: the title is saved, the visibility refused, the tool returns an error and `notify_epic_changed` is skipped. In the TUI epic editor (`src/runtime/editor.rs`) the feed take-over prompt is skipped too. Fix: one module reducer per entity that takes the patch, workspace and visibility, mirrored in the twin, with a conformance case "refused move leaves the title".

**R4. Low — fix for G1/G4 → the close path still runs on every tree exit.** `src/agent_tree/run.rs::DiffPaneTeardown`, `diff_pane.rs::mark_content`. *Verified by the reviewer.* A failed editor mark is logged and swallowed; on `q` the teardown reads the lower pane as a diff and kills the editor with its unsaved edits (`EditorPaneOutlivesItsTree`). Fix: honour a flag set by `record_editor_took_over`, or treat a failed mark as "editor".

**R5. Low (plausible) — fix for P1 not applied to its sibling.** `src/feed/exec.rs::run_feed_command_bounded` also uses `process_group(0)` but sets no non-interactive environment, so a feed command that prompts for an ssh passphrase is stopped by SIGTTIN and reported as a timeout. Fix: share the environment with `run_bounded` once R1 is fixed.

**R6. Low — stale doc comments.** `src/service/tasks/crud.rs::TaskService::update_task` still says "`patch_task` and `set_task_epic` recalculate"; `src/tui/update/admin.rs::handle_admin_open` and `src/tui/messages/admin.rs::AdminMessage::Open` say `W`, the key is `A`; `src/store/mod.rs::RepoConfigStore::set_verify_command` describes insert-on-unknown and whitespace-to-None, which the reducer does not do (see C5). *Verified.*

**R7. Low — the N2 test copies its oracle.** `spacetime/module/src/user_profile.rs` test `every_format_character_and_line_separator_is_refused`: its `REFUSED` table is a copy of `FORMAT_RANGES`, so a range missing from both passes. Fix: an independent oracle (a general-category crate as a host-side dev-dependency).

**R8. Low — fix for T2 → lost diagnostics.** `src/runtime/tests/live_store.rs::LiveStore::start` retries any early exit five times and reports "no store stayed up on a free port", hiding the child's stderr. Fix: retry only on address-in-use; otherwise fail with the exit status and stderr.

### 3.3 Board, admin screen and MCP

**B1. Medium — the admin screen hides every refusal.** `src/tui/ui/kanban/mod.rs::render` draws `render_error_popup` (line ~287) before `render_admin_overlay` (~295), and the overlay clears its 80% area. *Verified.* Deleting a non-empty workspace, adding an unknown name or revoking the last super-admin sets the error popup, which the user never sees; the next key is swallowed as "dismiss". `admin-surfaces.allium` says every refusal shows. Fix: draw the error popup last; add a render test of a refusal over the admin screen.

**B2. Low/Medium — the board decides membership before asking the store.** `src/mcp/handlers/epics.rs::unknown_workspace` (used by `handle_create_epic`, `handle_create_work_package`) and `PeopleStore::admin_snapshot`. *Verified by the reviewer.* Before the identity settles or a new membership row arrives, `my_workspaces` is empty and the agent is told "You belong to no workspace", which the store would not say (`NameWorkspaceAndVisibilityViaMcp`: every refusal is the store's). Fix: send the create and use the local list only to decorate the store's refusal; say "identity not settled yet" when it is not.

**B3. Low/Medium — the admin screen keeps cursors by position, not by id.** `src/tui/update/admin.rs::App::handle_admin_loaded`. *Verified by the reviewer.* Only the workspace cursor is restored by id; members, super-admins and unseen keep their index while lists re-sort, so after granting "Aaron" the highlight is on someone else and `v` targets them. A new workspace is not highlighted. Spec: `AdminScreenRefreshesAfterAWrite`, `CreateWorkspaceFromScreen`. Fix: restore each pane by its key; select a created workspace by name.

**B4. Low — `Z` on the board filter's Epics tab moves the cursor and ignores a standing query.** `src/tui/update/board_filter.rs::App::handle_toggle_all_filter_epic_folds`. *Verified by the reviewer.* `clamp_epic_cursor` keeps the index; since 00ef8606 the first `Z` unfolds all, so the cursor lands on a different row. With a title query, `Z` clears `open_epics`. Fix: remember the cursor's `EpicId` and find it again; make `Z` a no-op during a query.

**B5. Low — `set_super_admin` and `set_member` skip the store on a cached no-op.** `src/service/people.rs::PeopleService::set_super_admin`. *Verified by the reviewer.* A non-super-admin "granting" an existing super-admin gets success unasked; a revoke can be skipped against a stale cache. Fix: always call the reducer.

**B6. Low — small spec/code mismatches.** `src/store/queries/people.rs::admin_snapshot` orders unseen items by (kind, id), spec says workspace, kind, id; `src/models/people.rs::resolve_person` reports "ambiguous" where `PersonResolution` resolves by name; "Added X" vs spec "Added <name> to <workspace>". Fix the code or the spec for each.

**B7. Low (latent) — `SharedRows::retire_task` leaves undecodable rows counted.** `src/sync/rows.rs::SharedRows::retire_task` returns early when the departing row does not decode and never clears `undecodable_tasks`. Matters once tasks sit behind a view (#98691). Fix: clear the entry when no decoded row remains.

### 3.4 Fresh area: `src/cli/` and `src/mcp/trajectory.rs`

**C1. High — `store import` stops a running board's store.** `src/cli/store_import.rs::import_store` with `src/spacetime/managed_store.rs::ManagedStore::bring_up` and `::stop_on_exit`. *Verified.* When a managed board already runs on 127.0.0.1:3000, `bring_up` takes the `Adopt` branch and sets `held = true`; at the end the import calls `stop_on_exit`, which stops a held store. The board loses its store mid-session. `startup.allium: ImportBringsUpTheManagedStore` says the import leaves "the machine as it found it". Fix: stop only a store the import spawned (`stop_spawned_store`), or refuse when the address already answers and name `--spacetime-server`. Add a test with the probe answering.

**C2. Medium — `repo set-verify` writes the path before the command is checked.** `src/cli/commands.rs::set_verify` calls `save_repo_path`, then `set_verify_command`, which the module refuses on a newline. *Verified.* `dispatch repo set-verify /typo "a\nb"` exits 1 but leaves a new repo-path row. `tests/cli.rs::dispatch_repo_set_verify_rejects_newline` checks only the message. Fix: one reducer that saves and sets in one transaction (or validate first, as `handle_set_verify_command` does), and assert nothing was written.

**C3. Medium — `repo set-verify` takes over another host's repo row.** `config.rs::save_repo_path` stamps `host` on update too. *Verified (code path).* Host B sets a verify command for a path only host A has; the row's host becomes B, and B's next `prune-repo-paths` deletes it as missing, with A's verify command. Breaks `PruneRepoPathsViaCli`'s "never judge another host's row". Fix: do not re-stamp `host` on an existing row from `set_verify`.

**C4. Low (plausible) — prune deletes repos on an unmounted disk.** `src/cli/commands.rs::prune_repo_paths` uses `Path::exists()`, false on a permission error or a detached disk. Fix: delete only on `ErrorKind::NotFound` from `try_exists`.

**C5. Low — `clear-verify` and blank commands disagree with the docs.** `src/store/mod.rs::RepoConfigStore::set_verify_command` doc vs the reducer: `repo clear-verify /unknown` fails with "no repo path"; `repo set-verify p "  "` stores a blank command and prints success. Fix: correct the doc and reject or trim blank commands.

**C6. Low — the import's scratch store is not reaped and lives in `/tmp`.** `src/cli/store_import.rs::read_data_dir_with`: SIGTERM with no wait or KILL, then the TempDir is dropped while the store may still write; `store.log` is deleted on a start failure; a full copy of the old store goes to `/tmp` (RAM on some hosts). `data_dir_in_use` compares a canonical `--from` with the raw `--data-dir` text and misses a `spacetime start` with the default data dir. Fix: wait then KILL, keep the log on failure, put the scratch dir on disk, canonicalise both sides.

**C7. Low — trajectory files grow without bound.** `src/mcp/trajectory.rs::append_entry` writes full tool results to `trajectories/<task_id>.jsonl`; nothing prunes or reads them. Fix: cap each `result` and delete the file with the task (or by age).

**C8. Low — docs and help drift.** `docs/reference.md` "CLI Usage" claims to list every subcommand but omits `spacetime dump-server|restore` and `store import`; the `prune-repo-paths` help omits "only rows this host saved"; `docs/invariants.md` and `docs/conventions.md` cite `cmd_plan` → `TaskService::attach_plan`, but the CLI path is `src/cli/commands.rs::attach_plan` (`cmd_plan` in `src/main.rs` delegates to it). Fix the text.

**C9. Low (plausible) — the status line can exit non-zero.** `src/cli/statusline.rs::run` and `main.rs::cmd_caller_headers` use `print!`, which panics on a closed pipe; `StatusLineDecorator` promises exit 0. Fix: `let _ = write!(io::stdout(), …)`.

## 4. Hotspots and agent context

**Hotspots** (`git log --since=60.days -- src spacetime/module/src`, change count, current lines). First baseline for this skill; no earlier list to diff against.

| File | Changes | Lines |
|---|---|---|
| src/spacetime/module.wasm.source-hash | 81 | 1 |
| src/spacetime/module.wasm | 74 | binary |
| src/runtime/mod.rs | 71 | 1,258 |
| spacetime/module/src/lib.rs | 53 | 282 |
| src/tui/mod.rs | 51 | 922 |
| src/service/tasks/crud.rs | 50 | 911 |
| src/sync/writes.rs | 48 | 710 |
| src/runtime/tasks.rs | 47 | 1,254 |
| src/spacetime/bindings/mod.rs | 46 | 3,530 (generated) |
| src/tui/types.rs | 45 | 576 |
| src/store/mod.rs | 44 | 1,335 |
| src/mcp/handlers/dispatch.rs | 44 | 1,058 |
| src/db/mod.rs | 44 | removed |
| src/mcp/handlers/tests/mod.rs | 43 | 1,699 |
| src/models/tasks.rs | 41 | 1,178 |
| src/sync/tests/writes.rs | 40 | removed |
| src/runtime/tests/mod.rs | 40 | 972 |
| src/runtime/tests/misc.rs | 39 | 1,280 |
| src/dispatch/tests.rs | 37 | 448 |
| src/tmux.rs | 33 | 1,688 |

Busy and large: `src/runtime/mod.rs`, `src/runtime/tasks.rs`, `src/store/mod.rs`, `src/models/tasks.rs`, `src/mcp/handlers/dispatch.rs`. This run read only their in-range diffs, not the whole files; no finding beyond R6/C5. Note for the next run: the busiest module files are not in this list because the module is split by concern; `spacetime/module/src/*` churn is spread across `visibility.rs`, `tasks_epics.rs`, `workspace.rs`.

**Agent context** (`wc -w`; at `00ef35ab` → now): `CLAUDE.md` 2,298 → 2,281; `docs/invariants.md` 1,484 → 1,484; `docs/conventions.md` 12,974 → 13,029; `docs/reference.md` 9,757 → 9,675; `docs/testing.md` 4,403 → 4,424; `docs/module-map.md` 4,826 → 4,939. All `CLAUDE.md` + `docs/*.md`: 43,359. Flat; no action.

Five `docs/invariants.md` claims checked against code: `McpState::db` is `Arc<dyn TaskReadStore>`; `every_store_mutation_reaches_the_transport`, `dispatch_links_no_sqlite` and `lower_layers_do_not_import_higher_ones` exist where cited; `cached_epic_stats` self-heals. All hold; the one drift is the `cmd_plan` citation (C8).

## 5. Standing items

- **W2** — adopt unowned repo paths (`host == ""`) on prune. Open since 2026-10-22, optional. Ask: file or drop.
- **S2/S3 residue** — the service still reads the subscription cache for `close_pr_on_done` and pre-checks epics before `move_task_to_epic`/`create_task_inner`. Raised 10-22, 10-28 and now. Ask: file or drop.
- **T2 residue** — `startup_smoke.rs::free_address` binds and drops a port. Raised 10-28 and now.
- **Read enforcement** — every table is still `public` until #98691 lands, so all read-side visibility is advisory. Not a finding; context for V1–V12.

**Graduate to a check.** "A move path forgets a rule its sibling has" appeared in N1 (10-28) and R2 now, and "one logical change, several store writes" in S1 (10-22, 10-28) and R3 now. Deliverables: a conformance test that runs every task-move reducer (set_task_epic, update_task, relink via reroute/regroup/flatten, feed move) against the same unseen-mark and sight assertions; and a `scripts/check-*.sh` (or test) that flags a `src/service/` method making more than one `self.db.<write>` call outside an allow-list.

## 6. Areas read

- Every diff in `00ef35ab..e68f86a1`.
- In full: `spacetime/module/src/{visibility,workspace,seed,grouping,unseen,learning,packages,config,views,user_profile}.rs` and their twins in `src/sync/memory_caller/`; `src/service/{epics,people,work_packages}.rs`; `src/tui/update/admin.rs`, `src/tui/update/board_filter.rs`, `src/mcp/handlers/admin.rs`.
- Fresh area: `src/cli/` (all modules), `src/mcp/trajectory.rs`, `src/main.rs` subcommand wiring, `src/store_connection/mod.rs`, `docs/specs/cli.allium`.
