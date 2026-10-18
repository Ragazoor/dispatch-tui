# Codebase review — 2026-10-18

Task #94227 (epic #57348). This is a follow-up to the 2026-10-17 review
(`docs/plans/2026-10-17-codebase-review.md`). Twelve work packages from that
review landed in ~25 commits (`722eb9f2~1..HEAD`, 140 files, +10.9k/−0.7k
lines). This pass audits those fixes for gaps and regressions, and re-checks
every old finding, including the Low ones. Four read-only reviewers split the
code by area; the coordinating agent re-read the two items marked *Verified*.
`cargo clippy --all-targets -- -D warnings` is clean. Nothing else was run.

**Confidence key.** *Verified*: re-read in the code by the coordinating agent.
*Confirmed*: the reviewer traced it in source. *Plausible*: the path exists
but depends on timing, platform or trust-model questions.

## Executive summary

- **The fixes hold.** Of the 63 old findings, nearly all are fixed in the
  code, including every High. No Medium or High defect remains in the module,
  sync, store or service layers.
- **One fix is incomplete at the edge that matters most.** The new MCP
  Host/Origin guard ignores the port, and the hook route now accepts
  `text/plain` bodies, so a local web app can forge hook events.
- **Several fixes are partial.** Push-failure classification still treats
  hook/protected-branch rejections as retryable; `window_target` still reads a
  tmux error as "no windows"; the atomic writer breaks symlinked dotfiles.
- **Two small bugs are new.** The task-detail scroll bound double-counts the
  header separator. `push_hint_spans` byte-slices a label.
- **Test gaps follow the new branches.** Several refusal branches in the
  module and its in-memory twin, and the origin guard's edge cases, have no
  test.

## Findings

Paths are relative to the repo root.

### Medium

1. **Hook route dropped its Content-Type requirement** —
   `src/mcp/handlers/hooks.rs:~40` (`handle_hook_body`), wired at
   `src/mcp/mod.rs:~244`. *Confirmed.* Since 25c36a82 the handler takes raw
   `Bytes`; the old `Json<HookRequest>` extractor demanded
   `application/json`. A cross-origin `text/plain` POST is a CORS "simple
   request" and needs no preflight. The route needs no identity headers (the
   task id is in the body), so a page whose Origin passes the guard can forge
   `observe` events (Stop, needs_input) for any task. Fix: reject a
   non-JSON Content-Type, or take `Json<Value>` and decode afterwards.

2. **Origin guard ignores the port** — `src/mcp/middleware.rs:38`
   (`is_loopback`). *Verified.* `Origin: http://localhost:<any port>` passes,
   so any other local web app (a dev server on `:3000`) reaches the hook and
   pane-view routes. `/mcp` is still protected because `X-Caller-Kind` forces
   a preflight. The parser is also loose: `http://localhost:x@evil.com` reads
   as host `localhost` (browsers never send that form). Fix: compare the port
   with the bound one and parse the authority properly. Add tests for
   `Origin: null`, `localhost.`, upper case and a port mismatch.

3. **Task-detail scroll bound double-counts the separator** —
   `src/tui/ui/kanban/popups/task_detail.rs:~95`. *Verified.*
   `header_height` already includes the separator row (the header block's
   BOTTOM border, `Constraint::Length(header_height)`), so the body is
   `inner.height − header_height = height − 2 − header_height`. The code
   subtracts `2 + header_height + 1`. `max_scroll` is one too large and the
   last line stops one row above the bottom, against the claim in 606857e6.
   Fix: drop the extra `+ 1`. Add a test that the last line sits on the bottom
   row at `max_scroll`; `zoom_after_scrolling_to_end_keeps_last_line_visible`
   asserts only visibility.

4. **Push failures from hooks and protected branches are still "retryable"**
   — `src/repo_sync.rs:346` (`is_push_rejection`). *Verified by string
   logic.* The markers `"rejected"`, `"non-fast-forward"` and `"fetch first"`
   also match `! [remote rejected] … (pre-receive hook declined)` and
   `(protected branch hook declined)`. These become `PushRejected` again,
   which is what old finding 9 meant to stop. Fix: match `[rejected]`
   (without `remote`) or the two phrases only, and send `[remote rejected]` to
   `SyncError::Other`. `src/repo_sync/tests.rs:654` asserts only a generic
   string; add auth, network and hook cases.

### Low

5. **`window_target` still reads any `list-panes` failure as "no windows"** —
   `src/tmux.rs:~187`. *Verified.* Same bug class as old finding 10. The
   comment (~179) and the `rename_window` doc (~961) still claim it matches
   `list_all_window_names`, which now returns `Err`. A transient tmux error
   makes `rename_window` or `split` say "no tmux window named X" for a live
   window.

6. **Remaining soft-fail callers of `list_all`** — `src/dispatch/agents.rs:789`
   (`resume_agent`), `src/tmux.rs:234` use `has_window().unwrap_or(false)`; a
   transient error looks like "absent" and resume can create a duplicate
   window. `is_no_server` also matches any `error connecting to`, including
   "Permission denied". *Verified.*

7. **Atomic write replaces a symlinked config file** —
   `src/atomic_write.rs:48` (`temp.persist`). *Verified in code; the dotfile
   case is Plausible.* If `~/.claude.json` or `~/.claude/settings.json` is a
   symlink (dotfile managers), the rename swaps the link for a regular file;
   the real target goes stale. Hard links break the same way. Fix: write to
   the canonical target; add a symlink test.

8. **Concurrent-update loss is not fixed by the atomic write** —
   `src/dispatch/trust.rs` (`trust_at`), `src/setup/mod.rs`
   (`write_json_file`). *Confirmed.* Atomic rename removes truncation and the
   empty file after a crash. Claude Code writing between our read and our
   rename is still overwritten. Probably acceptable; the WP-3 plan overstates
   it. Narrow the window (read just before write) or say so in the docs.

9. **`add_worktree` classifies errors by English stderr** —
   `src/dispatch/worktree.rs:~650` (`stderr.contains("already exists")`).
   *Plausible.* A non-English `LANG` turns the existing-branch case into a hard
   failure. The same text matches `fatal: '<path>' already exists`; then
   `merge-base --is-ancestor` fails and the operator sees a misleading
   "refusing to reset branch… unpushed commits". Fix: `rev-parse --verify
   refs/heads/<name>` first, or run git with `LC_ALL=C`.

10. **`pull_base` aborts blindly and ignores the abort result** —
    `src/dispatch/finish.rs:~150`. *Confirmed.* `merge --abort` runs on any
    pull failure, including a network failure where no merge began, and its
    result is dropped. After a timeout that left `index.lock`, the abort fails
    silently and the checkout stays wedged. Check that a test covers the
    ahead-branch refusal in `src/dispatch/tests/worktree_safety.rs`.

11. **`ensure_worktrees_excluded` is not atomic and skips `.git` files** —
    `src/dispatch/worktree.rs:~602`. *Plausible.* Plain `fs::write` on
    `info/exclude`; repos whose `.git` is a file (submodules, a linked
    worktree used as repo path) keep the `?? .worktrees/` dirty state.

12. **Feed exec edge cases** — `src/feed/exec.rs`. *Verified.* A stdout read
    error returns without killing the process group, so `kill_on_drop` reaches
    only `sh`. Stderr stops at 32 MiB without killing anything. The group kill
    shells out to `kill(1)` synchronously inside an async fn.

13. **Failed editor-window create can kill a pre-existing window** —
    `src/runtime/editor.rs` (`clear_session_slot`). *Plausible, low.* The
    `EditorSession` has `cleanup_runner: Some`, so its Drop kills
    `window_name`. If the create failed because that name already existed, the
    other window dies. Names come from `new_window_name()`, so a collision is
    unlikely.

14. **`exit_session` still says "Unknown caller task N" for a mistyped target
    id** — `src/mcp/handlers/tasks/wrap_up.rs:353`. *Verified.* Old finding 45
    was fixed only in `record_learning`. `fetch_caller_task`, `resolve_list_scope`
    and `create_task` still return a raw JSON-RPC error, a different shape from
    the `isError` results unified in 25c36a82.

15. **`push_hint_spans` slices a label by byte** — `src/tui/ui/shared.rs:472`
    (`&label[1..]`). *Confirmed.* `can_embed` uses `char::eq_ignore_ascii_case`,
    which is true for identical non-ASCII chars; a key `å` with label `å…`
    slices mid-codepoint and panics. Latent while bindings are ASCII. Use
    `strip_prefix`.

16. **Managed store adopts any HTTP 200 on `/v1/ping`** —
    `src/spacetime/managed_store.rs:281` (`probe_address`). *Verified.* Old
    finding 60 is still open; e4bc043d fixed only the post-spawn wait.

17. **Module rule changes without a `RULES_VERSION` bump** — eb73e7eb moved
    `patch_task`'s `epic_id` refusal, the sub-status legality check and the
    `close_task` dispatch fail-open into the module; both constants stayed at 9.
    *Plausible.* An older board still passes the gate and will not expect the
    new behaviour. Decide whether the constant's rule requires a bump.

18. **Wrong comment in `rules.rs`** — `check_task_patch`. *Verified.* It says
    `set_task_epic` owns "the epic-exists check". `set_task_epic`
    (`spacetime/module/src/tasks_epics.rs:435`) and `route_to_repo_group` check
    existence only when routing a grouped root, so a nonexistent `epic_id` is
    written as a dangling id. Decide: add the check or fix the comment.

19. **In-memory twin lacks rollback in `patch_task`** —
    `src/sync/memory_caller/tasks_epics.rs` (`apply_patch_task`). *Plausible.*
    After `write_task` succeeds, `reroute_on_repo_change` can return `Refused`
    with the row already written; the module rolls the transaction back.
    Validation also runs before the later phoenix/stop-pending adjustments.

20. **`flatten`/regroup deletes skip `validate_epic_deletable`** —
    `spacetime/module/src/grouping.rs:118`. *Plausible.* The depth cap can
    still skip silently; the subtree there is shallow in practice.

21. **Uninstall prompts error out on EOF** — `src/setup/uninstall.rs:50`,
    `:192`. *Confirmed.* Safe (EOF is not consent) but noisy; the startup and
    host paths decline cleanly.

22. **Task delete discards the tear-down of an open `exit_session` row** —
    `delete_task_side_effects`. *Verified.* The spec
    (`docs/specs/store-split.allium`, ~1671) records it as deliberate.
    Listed so nobody rediscovers it.

### Test gaps

- `respawn_phoenix_successor` non-phoenix refusal
  (`spacetime/module/src/tasks_epics.rs:485`, mirrored at
  `src/sync/memory_caller/agent_state.rs:517`): no test on either side.
- Over-deep subtree refusal in `delete_epic`/`batch_delete`: tested only
  through the pure `subtree_exceeds_depth`; add a reducer and conformance test.
- `render_scroll_indicators` into a zero-size or one-row area; `body_height`
  on a short terminal.
- Origin guard edge cases (finding 2) and non-JSON hook bodies (finding 1).

### Open question: the trust model (unchanged)

Still undecided and still not decomposed: restore/admin reducers without
caller checks, `subscribe_to_epic` not comparing subscriber and caller,
`report_pr_status` without a host check, and MCP tools that ignore `identity`.
See the 2026-10-17 report. Deciding single-operator versus shared store would
settle all of them.

## Verified fixed (no defects found)

Old findings 2, 4, 5, 7, 8, 12–19, 21, 22, 25–31, 34, 35, 37–44, 46–50, 52–59
are fixed in code. Spot checks: `$1` positional launch quoting; EOF is an
error at prompts; `ensure_is_worktree`; `git_checked` in `dirty_files`;
`strip_prefix("refs/remotes/origin/")`; request-row retention keeps what
`check_exit_token` reads; the sync post-accept error path becomes `Dropped`;
`install` always clears rows; CVE feed paginates; clean-log feed exits 0;
release gates on CI; actions are SHA-pinned; `preserve_order` is on;
`module.wasm.source-hash` matches the sources; `RULES_VERSION` equals
`BOARD_RULES_VERSION` (9).

## Architecture and test health

- Layering (`tui` → `runtime` → `service` → `store` → `sync` → module) is
  intact and guarded by `tests/module_layering.rs`.
- Scale: 733 Rust files under `src/`; the largest are generated bindings
  (2,617 lines) and test files (up to 2,165). Largest production files stay
  at ~1,400–1,500 lines (`src/tmux.rs`, `src/dispatch/worktree.rs`).
- `spacetime/module-tests` is wired into CI and the pre-push hook through
  `scripts/check-spacetime-module.sh`. The doc checkers pass.
- The only sleeps in tests are deadline-bounded polls marked
  `allow-test-sleep`.

## CLAUDE.md and docs

- The three doc checkers pass; no drift found beyond finding 18's comment and
  the stale `window_target` comments (finding 5).
- Check that `docs/specs/` no longer says quick-dispatch gives "error for
  none" (606857e6 changed the doc comment to "picker otherwise, including none").
- Still missing: the store's trust model, the fact that decides the open
  question above.

## Prioritised actions

**Quick wins:** 3 (drop one `+ 1`), 5 (`window_target` error and its stale
comments), 14, 15, 16, 18, 21.

**Small efforts:** 1 + 2 (Content-Type and port check, with tests), 4
(`is_push_rejection` markers and cases), 7 + 8 (atomic writer on symlinks;
docs), 9 + 10 + 11 (worktree and git error handling), 6 + 13 (remaining tmux
soft-fails), 12 (feed process group).

**Module and twin:** 17 + 19 + 20 and the test gaps, in one work package.

**Decision needed:** the trust model.
