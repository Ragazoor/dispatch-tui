---
name: repo-health-review
description: >-
  Recurring health review of the dispatch repository itself: audits the last
  review's findings and fixes, hunts for defects in the commits since, reads one
  area no recent review has read, and ranks hotspots from git history. Use when
  the recurring Codebase Review task runs in this repo, or when the user asks for
  a codebase review, repo health check or code audit of dispatch — prefer it here
  over the generic kognic-code-quality:codebase-review, whose whole-repo
  checklist stopped yielding new findings on this codebase.
---

# Repo Health Review

A review of this repository that builds on the previous one instead of starting
from zero. Each run is anchored to a commit range and to the previous report.

Why this shape: across twenty runs (April to October 2026) the generic sections
(largest files, code smells, "top 3 changes") repeated themselves, while the
findings that mattered were defects in new commits, regressions in the previous
round's fixes, authorisation gaps, partial writes, races and twin-vs-module
drift. This skill asks for those directly.

## 0. Anchor the run

1. Find the previous report: the newest `docs/plans/*-codebase-review.md`.
   Older ones also live under `docs/plans/archive/`.
2. Read its header for the commit range it covered and its "Areas read" and
   "Standing items" sections.
3. Your range is `<previous range end>..main`. Record it, with the commit count,
   in your report's header. The next run depends on it.

If no previous report names a range, or its end is not on `main`
(`git merge-base --is-ancestor <sha> main` fails, e.g. the report was written
on a branch that was later rebased), use the commits of the last 14 days and
say so.

If the newest report is dated today (another review task already ran), stop and
ask the user before writing: a short follow-up over the new commits, or a full
pass that replaces it. Both reports would claim the same filename and the same
`docs/plans/review-<date>/` folder. On 2026-10-31 the user removed the first
report's tasks and chose a full replacing pass.

If the previous report has no "Standing items" section, seed one: list every
finding that appears in two or more of the last three reports and is still open.

## 1. Last round's findings: status table

For every finding in the previous report, check today's tree and mark it
**Fixed**, **Partly fixed**, **Open**, **Moot** or **Moved to standing items**.
One row per finding, with the commit that fixed it where there is one. Report the
yield line: "N of M fixed".

## 2. Regression audit of the fix commits

This step has found the most real defects. For every commit in the range that
fixed a previous finding, read the diff and ask:

- Did the fix make the same mistake in a sibling path (another reducer, the
  epic version of a task fix, the other handler)?
- Did it add a partial write: more than one store write where the rule needs one?
- Did it widen authority or skip a sight/visibility check?
- Was the module change mirrored in `src/sync/memory_caller/` and covered in
  `tests/memory_caller_conformance.rs`?
- Did it override user or environment configuration it should have kept?

Name a regression's chain ("fix for X → regression Y") so the next run can see it.

## 3. Defect hunt over the new commits

Read every non-fix commit in the range. Use these lenses, in this order:

1. **Authority and sight.** Who may call it? Does the module trust a
   client-supplied identity, owner or creator? Can a move or link hide someone
   else's work?
2. **Atomicity.** One logical change, one reducer call. Look for multi-step
   writes that leave a half-applied state on failure.
3. **Concurrency.** Two hosts, two agents, or a reactor racing a user action.
   Process groups, file writes that are not atomic, tmux targets without `-t`.
4. **Module vs twin vs spec.** Rule differences between `spacetime/module/src/`
   and `src/sync/memory_caller/`; behaviour the relevant `docs/specs/*.allium`
   file does not say. When the two specs disagree, report the disagreement, not
   a code fix.
5. **Live-data hazards.** Migrations, `RULES_VERSION` bumps, backfills that can
   skip rows already in the live store.
6. **Gates.** Is `main` green on `cargo test --no-fail-fast` and
   `cargo clippy --all-targets -- -D warnings`? Run both and report the result.
   Wrap-up rebases land commits on `main` without the pre-push hook, so a red
   gate is plausible (clippy failed on `main` on 2026-11-04). Before blaming
   the toolchain, check whether the last run was green on the same `rustc
   --version`; if it was, find the commit and fix it in this task.

## 4. One fresh area

Pick one directory or module that none of the last three reports lists under
"Areas read". Prefer one with recent churn (step 5). Read it as a defect hunt
with the lenses above. Name the area in your "Areas read" section.

## 5. Hotspots from git history

Rank files by how often they change, not by size alone:

```bash
git log --since=60.days --format= --name-only -- src spacetime/module/src \
  | sort | uniq -c | sort -rn | head -20
```

Cross the top 20 with line counts (`wc -l`). Read the top 5 that are both busy
and large, and report only what changed since the last run's list. A file that
is large but untouched is not a finding.

## 6. Agent context

Agents read `CLAUDE.md`, `docs/invariants.md` and the skills on every task.
Check:

- Size trend: `wc -w CLAUDE.md docs/*.md` against the last report's figures.
- A claim in the docs the code no longer makes true. Check a sample of five
  statements in `docs/invariants.md` against the code.
- Rules a hook or `check-*.sh` script already enforces, restated in prose.

## Rules for findings

- **Evidence.** Every finding names a path and symbol, a severity
  (High/Medium/Low), and a concrete failure scenario. Mark it *Verified* (you
  re-read the code) or *Plausible*.
- **No repeats.** A finding the previous report raised and that is still open
  goes in the status table, not again as a new finding.
- **Standing items.** A finding raised in two reports with no action, or a design
  decision waiting on the user, moves to a "Standing items" section with one line
  each. Do not raise it again as a finding. Ask the user once whether to file it
  as its own task or drop it (on 2026-11-03 the user chose one package for two
  long-carried items; on 2026-11-04, its own package for T2). A standing item whose task is Done is not closed until the
  tree shows it: on 2026-11-03 two were still open after their tasks closed.
- **Graduated checks get audited too.** Mutation-test a check-*.sh or guard
  test that a previous round added: re-introduce the original bug and confirm it
  fails. On 2026-11-03 both such checks passed green while missing their class.
- **Graduate repeats into checks.** When the same *class* of finding appears in
  two runs, the deliverable is a mechanical check (a clippy lint in `Cargo.toml`,
  a `scripts/check-*.sh`, or a test), not another one-off fix.
- **Cap.** Rank findings by severity, then by how busy the file is. Low findings
  in untouched code can be left out.

## Output

Write `docs/plans/<YYYY-MM-DD>-codebase-review.md` with:

1. Header: task id, previous report, commit range and count, what was run and
   the gate results (step 3, lens 6).
2. Executive summary: 3–5 bullets.
3. Status table of last round's findings (step 1).
4. Findings, grouped by area, each with an id (e.g. `V1`, `R1`), severity,
   path and symbol, scenario and fix.
5. Hotspots and agent context (steps 5 and 6): the ranked top-20 list with
   change counts and line counts, and the `wc -w` figures, then what changed.
   Record the absolute figures, not only the deltas, so the next run has a
   baseline.
6. Standing items.
7. Areas read: every module this run read in full.

The findings section is the input for `/decompose-review`, so keep each finding
self-contained.
