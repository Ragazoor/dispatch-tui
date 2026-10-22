# Review Cleanups

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Close the Low findings from the 2026-10-22 review in one pass.

## Context

This work package addresses findings from `docs/plans/2026-10-22-codebase-review.md`. Behaviour-changing items start in the spec. Do not touch `src/dispatch/split_panes.rs` or `src/runtime/mod.rs`; packages 1 and 3 own them.

## Findings

### Low: `tui` has no layering row and reaches into `dispatch` twice (A1)

`tests/module_layering.rs:21`; `src/tui/mod.rs:425` (`crate::dispatch::DISPATCH_REQUEST_TIMEOUT`); `src/tui/update/forms.rs:144` (`crate::dispatch::validate_repo_path`).

**Fix:** Move the constant to `src/clock.rs` or `models`, and the validator to a leaf module. Then add `("tui", &["runtime", "tmux", "store", "sync", "spacetime", "mcp", "dispatch"])` to `RULES`.

### Low: Hard-coded MCP server version (A2)

`src/mcp/handlers/dispatch.rs:736` returns `"0.1.0"`.

**Fix:** Use `env!("CARGO_PKG_VERSION")`. Add a test that it matches.

### Low: Module map omits `restore_pinned_pane` (A3)

`docs/module-map.md:101` names two split entry points, not the third (`src/runtime/split.rs:84`).

**Fix:** Add it to the row, with the leftover-window rule.

### Low: Third copy of the sanitised git test helper (S3)

`tests/tmux_lifecycle.rs:155`, `tests/worktree_teardown.rs:27`, `src/dispatch/tests/worktree_safety.rs:9`. `docs/testing.md:138-145` names only two.

**Fix:** One helper in `tests/common/git.rs`; keep or move the `src/` copy. Update the `docs/testing.md` note.

### Low: CVE fix-PR matching can link a PR that only mentions the advisory (S5)

`scripts/fetch-cve.sh` (`$bot_prs` fallbacks: body `contains($id)`, then whole-token package name). Plausible, not observed.

**Fix:** Require the advisory id in title or body for the name fallback, or print the match reason in the `Fix PR:` line. Extend `scripts/test-fetch-cve.sh`.

### Decision: `RULES_VERSION` after eb73e7eb (F17)

`spacetime/module/src/rules.rs:52` and `src/sync/rules_gate.rs:21` are both 9. The constant's doc (`rules.rs:21-27`) says a rule moving into the module bumps it.

**Fix:** Bump both to 10 and rebuild with `./scripts/build-managed-module.sh`, or add a line to the doc saying why that move did not count. If unsure which, ask the user.

## Changes

| File | Change |
|------|--------|
| `tests/module_layering.rs` | Add the `tui` row |
| `src/clock.rs` or `src/models/` | Receive `DISPATCH_REQUEST_TIMEOUT` and `validate_repo_path` |
| `src/tui/mod.rs`, `src/tui/update/forms.rs` | Use the new locations |
| `src/mcp/handlers/dispatch.rs` | Crate version in `serverInfo` |
| `docs/module-map.md` | Name `restore_pinned_pane` |
| `tests/common/git.rs`, the three helper sites, `docs/testing.md` | One shared git helper |
| `scripts/fetch-cve.sh`, `scripts/test-fetch-cve.sh` | Tighter fallback match |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | Bump or document |

## Verification

- [ ] Run existing tests: `cargo test --no-fail-fast > scratch.txt 2>&1; echo $?`
- [ ] Run the hook's `check-*.sh` scripts and `scripts/test-fetch-cve.sh`
- [ ] If `RULES_VERSION` moved: both constants match and `build-managed-module.sh --check` passes
