# Learning Verdict Repeats

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Settle whether one task can rate the same learning more than once, and make the spec and code agree.

## Context

This work package addresses a finding from the code review in `docs/plans/2026-10-22-codebase-review-followup.md` (S3).

## Findings

### 🟢 Low: Repeated `rate_learning` calls inflate a learning's score (`src/service/learnings.rs:300`)

**Issue:** `apply_verdicts` (`src/service/learnings.rs::apply_verdicts`) only checks that the learning was retrieved during the task; `apply_learning_verdicts` in `spacetime/module/src/learning.rs` adds ±1 on every call. `RateLearningViaMcp` in `docs/specs/learnings.allium` is silent on repeats. Scenario: the task prompt says rate a learning "when you act on it", and the wrap-up `/learnings` step says to rate again, so one use gives +2. The `↑N` ranking and the stale-archive rule (`upvote_count <= 0`) both read that count. The retrieval check is itself a data rule living in `src/service/`.

**Fix:** See Decisions.

## Decisions

### One verdict per (task, learning), or repeats allowed (`src/service/learnings.rs:300`)

**Question:** Should a task's repeated verdicts on one learning count once or each time?

**Options:**
- **(a) One verdict per (task, learning), latest wins.** Record verdicts per (task, learning) in the module; a repeat replaces the earlier one and adjusts the count by the difference. Move the retrieval check into the reducer too. Spec first, module + twin + conformance, `RULES_VERSION` bump. Medium effort; fixes the score.
- **(b) Repeats are intended.** Write that into `RateLearningViaMcp` and tell agents in the `/learnings` skill and task prompt to rate once. Cheap; leaves the count gameable.

Ask the user with `AskUserQuestion` before implementing this finding. Do not pick an option yourself. If the user does not answer, leave the finding unimplemented and say so at wrap-up.

## Changes

| File | Change |
|------|--------|
| `docs/specs/learnings.allium` | Record the decision in `RateLearningViaMcp` |
| `spacetime/module/src/learning.rs`, `src/sync/memory_caller/`, `tests/memory_caller_conformance.rs` | Option (a) only |
| `src/service/learnings.rs` | Option (a): drop the retrieval check once the reducer owns it |
| `plugin/skills/learnings/SKILL.md`, task prompt | Option (b): say rate once |

## Verification

- [ ] Run the task's verify command — all pass
- [ ] `cargo test --no-fail-fast` green
- [ ] Option (a): a test rating the same learning twice from one task changes the count once
