# Loss of sight: strand refusal on every path, and marks parity

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** every path that takes a task out of a person's sight treats a live agent on their host the same way, and the conformance tests prove module and twin drop the same marks.

## Context

Findings L1, L2 and V5 from `docs/plans/2026-11-15-codebase-review.md` §3.2–3.3. L1 is the sibling A2's fix (69053de3) missed. Spec first (`docs/specs/workspaces.allium: ClaimsFollowSight` and the detach refusal), then tests, then code. Mirror module changes in `src/sync/memory_caller/`; bump both rules versions together.

## Findings

### 🟡 Medium — L1: "refuse while another person's host runs it" covers detach only (`spacetime/module/src/visibility.rs::check_task_move` non-detach branch, `::set_epic_visibility_row`, `::move_epic_row`; `tasks_epics.rs::patch_epic_row` re-parent; `workspace.rs::remove_member`)

**Issue:** Alice moves Bob's running task into her private epic, or makes its epic private: `on_task_sight_lost` releases Bob's claim, but his reactor still holds a live agent and open session rows; `get_task` answers None, wrap-up fails as "task deleted" and PR polling stops. The operator bypass in `check_task_move` also skips the check.

**Fix:** see Decisions. Either way, add conformance arms for each path.

### 🟢 Low — L2: loss-of-sight conformance checks claims, not marks parity (`tests/memory_caller_conformance/loss_of_sight.rs`, `bookmarks.rs`; `src/sync/memory_caller/tasks_epics.rs::apply_update_epic` comment)

**Issue:** mutation on 2026-11-15: removing the twin's mark drop in `on_task_sight_lost` passed all conformance tests (only a twin unit test caught it); a module-side omission would pass both. No "move into a private epic" arm. Marks after `move_epic_to_workspace`, `batch_delete`, `delete_epic` and member removal are uncovered (B4 residue). The twin's `apply_update_epic` comment says it never half-applies, which is false in the drift case.

**Fix:** after every loss-of-sight arm, compare the bookmarks and unseen marks each identity can read on both sides; add the missing arms; reword the comment.

### 🟢 Low — V5: the super-admin unseen-item delete names a task inside a private sub-epic (`spacetime/module/src/visibility.rs::super_admin_delete_unseen_item`; twin `src/sync/memory_caller/visibility.rs`)

**Issue:** `validate_epic_deletable` runs before `subtree_holds_a_seen_private_epic`; since 2c2279ca its refusal names the lowest-id undone task. `delete_epic` orders the checks the other way for this reason.

**Fix:** run the seen-private-subtree check first, on both sides; conformance case.

## Decisions

### L1: may a sight change strand another person's running agent?

**Question:** when a task move into a private epic, an epic going private, a re-parent, an epic's workspace move or a member removal would take a task out of sight of a person whose host runs its agent, should the store refuse, or allow it?

**Options:** (a) refuse on every path, as detach does (`task_detach_strands_a_host` over the moved subtree) — consistent and safe, but a holder can no longer make an epic private while someone else works in it; (b) allow it, and serve a task with a live agent or open request to its host regardless of sight — no refusals, but a person keeps reading a task they otherwise could not see; (c) allow it and accept the stranding (record it in the spec) — no code, but wrap-up and PR polling break for the stranded agent. The operator bypass follows whichever is chosen.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/workspaces.allium` | the decided rule for every loss-of-sight path |
| `spacetime/module/src/{visibility,tasks_epics,workspace}.rs` | apply the rule; swap V5's check order |
| `src/sync/memory_caller/` | mirror; fix the `apply_update_epic` comment |
| `tests/memory_caller_conformance/{loss_of_sight,bookmarks}.rs` | marks parity; missing arms; V5 case |
| `spacetime/module/src/rules.rs`, `src/sync/rules_gate.rs` | bump together |

## Verification

- [ ] `./scripts/verify.sh` passes
- [ ] Removing the twin's mark drop in `on_task_sight_lost` now fails a conformance test
