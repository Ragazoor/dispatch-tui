# Codebase review — 2026-10-17

Task #77835 (epic #57348). An unbiased defect hunt across the whole crate and
the SpacetimeDB module, done by six read-only reviewers in parallel (one per
area) and spot-checked by hand. Nothing was run except `cargo clippy
--all-targets -- -D warnings` (clean).

**Confidence key.** *Verified*: re-read in the code by the coordinating agent.
*Confirmed*: the reviewer traced it in source. *Plausible*: the path exists
but depends on timing, SDK behaviour or a trust-model question.

## Executive summary

- **The codebase is healthy at the surface.** Clippy is clean, coverage is
  gated at 86% (last 92%), ~6,400 tests, almost no TODOs, and the reviewers
  found no panic reachable from user input in the TUI or the MCP handlers.
- **The real defects sit at the edges:** shell quoting into `bash -c`, git
  error handling in the operator's primary checkout, and non-atomic writes to
  user-global files (`~/.claude.json`, snapshots, the store pin).
- **The MCP port trusts any local HTTP caller.** No Host/Origin check, no
  secret, and caller identity is a client-supplied header. A DNS-rebinding page
  could set feed or verify commands that the board and agents then execute.
- **The module has rule gaps:** two reducers write `sub_status` without the
  legality check, `patch_task` relinks epics without routing, and request
  tables grow forever.
- **Several reducers and tools lack caller checks.** That matters only if the
  store is shared by several people, a trust-model decision this review
  cannot settle (see "Open question" below).

## Findings

Ordered by severity within each group. Paths are relative to the repo root.

### High

1. **MCP port has no Host/Origin check or secret** — `src/mcp/mod.rs:237`
   (`router_over`), `src/mcp/middleware.rs:23`. *Plausible (verified: no check
   exists).* A DNS-rebinding page reaching `127.0.0.1:<port>` can POST
   `X-Caller-Kind: session` and call `update_epic(feed_command)`,
   `managed_feed_config`, `set_verify_command` or `dispatch_task`. Fix: reject
   requests whose `Host` is not `127.0.0.1:<port>`/`localhost:<port>` or that
   carry a non-loopback `Origin` (the MCP streamable-HTTP spec asks for this);
   optionally a per-board bearer token in the agent MCP config.

2. **Agent launch breaks on repo paths with a space or `'`** —
   `src/dispatch/agents.rs:363` (`prompt_launch_command`),
   `src/dispatch/caller_identity.rs:99` (`mcp_config_flag`). *Verified.* The
   shell-quoted `--mcp-config '<path>'` is spliced inside the single-quoted
   `bash -c '…'` body, so its quotes close the outer ones. A repo under
   `~/My Projects/` never starts claude; a `'` lets path text run as shell.
   Contradicts the one-quoting-layer claim on `claude_quoted`. Fix: pass the
   path as a positional (`"$1"`) like `$0`; add a test with a space and a `'`.

3. **`~/.claude.json` rewritten non-atomically, re-sorted, no backup** —
   `src/setup/mod.rs:132` (`write_json_file`, used by
   `src/setup/config_update.rs:22`) and `src/dispatch/trust.rs:36`
   (`trust_at`). *Verified.* `fs::write` truncates in place while Claude Code
   rewrites the same file constantly: a concurrent update is lost, a crash
   leaves it empty. `serde_json` lacks `preserve_order` (`Cargo.toml:29`), so
   every key is re-sorted on first rewrite. Fix: temp file + `persist` (as
   `src/host_file/mod.rs::write_whole` does), keep mode, enable
   `preserve_order`.

### Medium

4. **EOF at a yes/no prompt counts as "yes"** — `src/setup/confirm.rs:33`
   (`StdinConfirmer::prompt`). *Verified.* `read_line` → `Ok(0)` → `""` →
   default; the startup "Update it now?" defaults to yes. Ctrl-D or a closed
   stdin rewrites `~/.claude.json`, plugins and tmux.conf, contradicting
   `src/startup/config.rs:119`. Fix: treat `Ok(0)` as no/error in `confirm`,
   `confirm_dangerous`, `prompt_text`.

5. **Husk directory treated as a reusable worktree** —
   `src/dispatch/worktree.rs:521` (`worktree_is_reusable`), `:617`
   (`fast_forward_reused_branch`). *Verified (reuse check is dir-only).* A
   leftover non-git dir at `.worktrees/<id>-<slug>` skips `git worktree add`;
   `git -C <husk> merge --ff-only` then runs in the operator's primary
   checkout. Fix: require `<path>/.git` or `rev-parse --show-toplevel == path`.

6. **Wrap-up pull leaves primary checkout mid-merge** —
   `src/dispatch/finish.rs:150` (`pull_base`). *Verified.* No `merge --abort`
   on failure, unlike `rebase_branch` and `merge_origin_base`; the next
   wrap_up/sync is refused as dirty. Fix: abort on failure, or fetch +
   `merge --ff-only`.

7. **Default branch with a slash truncated** — `src/git.rs:178`
   (`detect_default_branch`). *Verified.* `rsplit('/')` turns `release/2024`
   into `2024`. Fix: `strip_prefix("refs/remotes/origin/")`.

8. **Git preconditions fail open** — `src/git.rs:288` (`dirty_files`), `:268`
   (`current_branch`). *Verified for `dirty_files`.* A non-zero
   `git status` exit reads as "clean". Fix: use `git_checked` and return
   git's stderr.

9. **Every push failure reported as "origin moved, retry"** —
   `src/repo_sync.rs:324` (`push_base`). *Confirmed.* Auth, network, hook and
   protected-branch errors become retryable `PushRejected`. Fix: classify on
   stderr / `--porcelain`; others to `SyncError::Other`.

10. **tmux non-zero exit read as "no windows"** — `src/tmux.rs:375`
    (`list_all`), `src/runtime/editor.rs:41`. *Confirmed.* A transient
    `list-windows` failure makes a live window look absent: the editor watcher
    ends, liveness reports a crash, teardown skips the kill. Contradicts the
    docs on `has_window_or_assume_present` and
    `MAX_CONSECUTIVE_QUERY_FAILURES`. Fix: return `Err` except for "no server
    running".

11. **Unbounded tmux calls on the command-drain thread** —
    `src/runtime/editor.rs:235` (`exec_pop_out_editor`). *Confirmed.* A
    wedged tmux server freezes the TUI. Fix: `spawn_blocking` or
    `run_with_timeout`.

12. **Session reactor drops tear-down rows before identity settles** —
    `src/runtime/session_host.rs:68` (`act`). *Plausible.* Only `serve` rows
    are released back; `tear_down` ids were already removed from
    `open_exits`, so the window is never killed. Fix: re-open them.

13. **Sync shows "Connected" with an empty view after a post-accept error** —
    `src/sync/session.rs:422` (`SyncSession::attempt`). *Verified.*
    `Accepted` is applied, then `?` on `adopt_user_identity_with_credential`
    / subscribe setup propagates; the next tick sees `Connected` and never
    subscribes. Fix: on any post-accept `Err`, apply `Dropped`, disconnect,
    return `Failed`.

14. **Unfollowing one epic can drop depth-2+ rows of another** —
    `src/sync/session.rs` (`reassert_on_shrink`),
    `src/sync/sdk_connector/mod.rs` (`reset_subtree`),
    `src/sync/sdk_connector/connect.rs` (`subscribe`). *Plausible — needs a
    live-store test.* Widenings are re-requested only from insert callbacks,
    which do not fire for cached rows. Fix: re-seed the walk from cached epics.

15. **Epic Done stamp computed client-side** — `src/service/epics.rs:394`
    (`EpicService::update_epic`). *Confirmed.* Unlike the task path, a stale
    cached status sends `completed_at = now` and overrides the module's stamp
    and any explicit reorder value. Fix: mirror `TaskService::update_task`.

16. **`create_epic` is create-then-patch** — `src/service/epics.rs:205`.
    *Verified.* A failed patch leaves an epic without feed command/interval/
    sort order, and a retry duplicates it; the doc claims otherwise. Fix: carry
    the fields in `encode::create_epic_row`.

17. **Two reducers write `sub_status` without the legality check** —
    `spacetime/module/src/tasks_epics.rs:516` (`batch_patch_sub_status`),
    `spacetime/module/src/agent_state.rs:443` (`record_pre_tool_use`), `:472`
    (`record_notification`). *Verified for the batch.* A stale view can store
    backlog/approved. Fix: validate in `write_task` (the one write seam);
    mirror in `src/sync/memory_caller/`.

18. **`patch_task` applies `epic_id` raw** —
    `spacetime/module/src/tasks_epics.rs:339`. *Confirmed.* Skips
    `route_to_repo_group` and the epic-exists check that `set_task_epic` does.
    Fix: route through `set_task_epic`'s logic or refuse `epic_id` here.

19. **Request rows never pruned** — `spacetime/module/src/dispatch_request.rs`,
    `spacetime/module/src/session_request.rs`. *Verified (only timer deletes
    exist).* Both public tables grow forever and are downloaded on every
    connect; `check_exit_token` scans a task's whole history. Fix: delete in
    `delete_task_side_effects`; prune old terminal rows.

20. **Failed managed-store start kills another board's store** —
    `src/spacetime/managed_store.rs:851` with `:741`. *Confirmed, rare.*
    `stop_managed_store` SIGTERMs every pid on the port. Fix: on a failed
    start, signal only the tracked child.

21. **CVE feed truncated at 100 alerts under Reconcile** —
    `scripts/fetch-cve.sh:58`. *Verified.* Alerts past 100 lose their tasks
    and worktrees. Fix: `gh api --paginate`.

22. **Log-warnings feed fails on a clean log** —
    `scripts/fetch-log-warnings.sh` (the `grep | awk | jq` pipeline).
    *Confirmed.* `grep` exit 1 under `pipefail`. Fix: `{ grep … || [ $? -eq 1 ]; }`.

23. **Feed timeout leaves orphaned children** — `src/feed/exec.rs:93`
    (`exec_feed_command`). *Plausible.* `kill_on_drop` kills `sh` only. Fix:
    `process_group(0)` + kill the group; cap stdout.

24. **Snapshot dump truncates the target first** — `src/main.rs:665`
    (`write_snapshot`). *Confirmed.* A failure mid-write destroys the previous
    backup; default umask on a full board dump. Fix: temp file + `persist`,
    mode 0600.

25. **Release builds do not wait for CI** —
    `.github/workflows/release-please.yml`. *Confirmed.* Fix: gate on CI
    success for the SHA.

26. **Shipped-script install error skips the manifest** —
    `src/setup/plugins.rs:548` (`install_one_shipped_script`). *Confirmed.*
    `?` on the prompt contradicts "never fails the run". Fix: map to
    Kept/Failed, always write the manifest.

27. **Stale-plugin cleanup follows symlinks** — `src/setup/plugins.rs:79`
    (`remove_stale_recursive`). *Confirmed.* A symlinked skill dir has its
    target's files deleted. Fix: `entry.file_type()`, unlink symlinks.

28. **Failed "mark done" returns a success-shaped result** —
    `src/mcp/handlers/tasks/crud.rs:305`, `:184`. *Confirmed.* No `isError`;
    the multi-id count says "3 of 3". Fix: return an error response.

29. **Omitted `arguments` rejected** — `src/mcp/handlers/dispatch.rs:744`.
    *Verified.* `Value::Null` fails every struct parse. Fix:
    `unwrap_or_else(|| json!({}))`, null too.

30. **Inconsistent MCP error shapes** — `src/mcp/handlers/dispatch.rs:750`
    vs `:815`. *Confirmed.* Name-resolution failures are raw JSON-RPC errors
    and skip trajectory/usage recording; the rest are `isError` with the code
    dropped. Fix: route both through `tool_error`.

31. **`set_verify_command` schema forbids the documented null** —
    `src/mcp/handlers/dispatch.rs:592`. *Confirmed.* Fix:
    `"type": ["string","null"]`.

32. **Task-detail scroll is miscounted and never clamped** —
    `src/tui/ui/kanban/mod.rs:522` (`wrapped_line_count`),
    `src/tui/ui/kanban/popups/task_detail.rs:101`. *Verified.* Byte length
    vs word-wrapped cells: non-ASCII over-counts, prose under-counts; zoom or
    resize leaves `scroll` past the new max. Fix: count display width with
    word wrap; clamp and write back.

32a. **`.worktrees/` never ignored in target repos** —
    `src/dispatch/worktree.rs:576` (creates `<repo>/.worktrees`),
    `src/git.rs:288` (`dirty_files`). *Plausible (verified: nothing writes
    `info/exclude` and `parse_porcelain_files` does not filter it).* In a repo
    whose own `.gitignore` lacks `.worktrees/`, `git status --porcelain`
    shows `?? .worktrees/`, so every wrap_up rebase and repo sync is refused
    as `DirtyPrimaryWorktree`. Fix: append `.worktrees/` to
    `<git-common-dir>/info/exclude` idempotently when creating the root.

### Low

33. `src/dispatch/worktree.rs:596` — `git worktree add -B` resets an existing
    branch that holds unpushed commits when the dir is gone.
34. `src/models/tasks.rs:929` (`slugify`) — no length cap; long or CJK titles
    exceed 255 bytes.
35. `src/dispatch/finish.rs:67` — doc says 60s, `SUBPROCESS_TIMEOUT` is 120s.
36. `src/tmux.rs:703` — `ensure_split_hook`'s doc attaches to
    `SPLIT_NEEDS_CORRECTION`.
37. `src/startup/store_pin.rs:49`, `src/startup/launch.rs:191` — non-atomic
    pin/record writes; a truncated pin aborts every launch.
38. `src/setup/config.rs:108` (`merge_mcp_config`) — non-object `mcpServers`
    reports `changed` forever.
39. `src/main.rs:490` (`cmd_verify_feed`) — no timeout; empty feed is exit 1.
40. `src/editor.rs:31` — `$EDITOR` split on whitespace, not shell quoting.
41. `src/host_file/mod.rs:34` — `HostIdentity` drops unknown fields on
    rewrite.
42. `.github/workflows/ci.yml` — actions pinned by tag; `cargo install
    cargo-tarpaulin` unpinned.
43. `src/mcp/handlers/tasks/mod.rs:201`, `src/mcp/handlers/epics.rs:44` —
    `limit` rejects numeric strings.
44. `src/mcp/handlers/learnings.rs:219` — negative `limit` becomes
    `usize::MAX`.
45. `src/mcp/handlers/types.rs:491` (`fetch_caller_task`) — "Unknown caller
    task" for a mistyped target id.
46. `src/mcp/handlers/task_refs.rs:29` — `is_id` trims, the parser does not.
47. `src/mcp/handlers/tasks/crud.rs:458` — `list_tasks` description says
    explicit scope disables exclusion; it does not.
48. `src/mcp/handlers/dispatch.rs:787` — usage rows recorded for unknown tool
    names.
49. `src/hooks/mod.rs:131` — no catch-all for unknown hook events across
    versions.
50. `src/sync/sdk_connector/connect.rs` (`disconnect`) — late callbacks can
    repopulate rows after `clear`; `install` does not clear again.
51. `src/sync/memory_caller/tasks_epics.rs` (`apply_patch_task`) — the double
    does not roll back on `Refused`.
52. `src/store/queries/mod.rs` (`ingest_feed_inner`) — swallows an identity
    error and stamps `created_by = ""`.
53. `spacetime/module/src/tasks_epics.rs:478` (`respawn_phoenix_successor`)
    — no phoenix-flag guard; check whether it is still called.
54. `spacetime/module/src/session_request.rs:992` (`close_task`) — does not
    fail open dispatch requests.
55. `spacetime/module/src/tasks_epics.rs` (`delete_epic_subtree`) — depth cap
    skips silently.
56. `spacetime/module/src/rules.rs:134` (`validate_epic_parent`) — returns
    `Ok` after the hop cap.
57. `src/spacetime/restore.rs:5` — cites removed `SeedSharedStore`; restore
    no longer refuses a non-empty store.
58. `src/spacetime/import.rs:221` — id conflict check covers four tables only.
59. `src/spacetime/managed_store.rs:829` (`stop_once`) — returns `true` after
    a timed-out first stop.
60. `src/spacetime/managed_store.rs:281` — any 200 on `/v1/ping` is adopted
    as ours.
61. `src/tui/ui/kanban/columns.rs:421` — direct buffer indexing on a
    zero-size area.
62. `src/tui/input/normal.rs:354` — quick-dispatch doc says "error for none";
    it opens the picker.
63. `src/tui/input/normal.rs:28` — unhandled `board.normal` action is a
    silent no-op.

### Open question: the trust model (not decomposed)

These are correct for a single-operator store and holes for a shared one.
The specs do not say which the store is. Decide before fixing:

- `spacetime/module/src/seed.rs`, `sequence.rs:33`, `:108`, `lib.rs:188` —
  restore/admin reducers (`seed_*`, `burn_id_sequence`, `set_schema_version`)
  have no caller check; `burn_id_sequence(…, i64::MAX)` loops in one
  transaction.
- `spacetime/module/src/config.rs:143` — `subscribe_to_epic` /
  `unsubscribe_from_epic` never compare `subscriber` with the caller.
- `spacetime/module/src/pr_status.rs:86` — no `host == task.host` check.
- `src/mcp/handlers/tasks/wrap_up.rs:196`, `:344`,
  `src/mcp/handlers/learnings.rs:132`, `src/mcp/handlers/tasks/dispatch.rs:19`
  — `identity` ignored; any agent can wrap up, exit or rate for another task.

## Architecture and complexity notes

- Layering (`tui` → `runtime` → `service` → `store` → `sync` → module) is
  consistent and guarded by `tests/module_layering.rs`. The in-memory double
  copies reducer bodies closely; the store/sync reviewer diffed the task,
  epic, dispatch-request and feed-ingest reducers and found no divergence.
- Size: 812 Rust files. ~83k lines of non-test code in `src/` (excluding
  generated bindings), ~130k lines of inline test code under `src/`, and ~20k
  in `tests/` — test code is about 1.8× production. Largest non-test files ~1,400–1,500 lines (`src/tmux.rs`,
  `src/dispatch/worktree.rs`, `src/dispatch/mock_sequence.rs`).
- Longest functions (~100–160 lines): `src/runtime/mod.rs::bootstrap_inner`,
  `run_tui`, `src/tui/ui/kanban/status_bar.rs::status_line`,
  `src/runtime/commands.rs::dispatch_task`,
  `src/service/tasks/dispatch.rs::serve_dispatch_request`.
- 289 `unwrap`/`expect` in non-test files (clippy-gated, mostly justified).

## CLAUDE.md and docs

- `docs/conventions.md` is 88 KB and `docs/reference.md` 65 KB; module-map
  table cells run to 3,000+ characters (`src/agent_tree/` row). Hard to scan;
  consider splitting long cells into per-directory subsections.
- CLAUDE.md lines reach 750 characters. Fine for agents, hard for humans.
- Missing: the store's trust model (single operator vs shared) — the one fact
  that decides a whole class of findings above.

## Prioritised actions

**Quick wins:** 2, 4, 6, 7, 8, 21, 22, 29, 31, 32, 35, 36.

**Larger efforts:** 1 (MCP origin/auth), 3 + 24 + 37 (one atomic-write helper
for user files), 5 + 33 (worktree reuse safety), 13 + 14 (sync reconnect),
17 + 18 + 19 (module rules, with memory-caller mirror), the trust-model
decision.
