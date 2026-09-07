# 4727 — Can the harness absorb this, so the prompt need not say it?

Task #4706 (report: `docs/plans/4706-prompt-audit.md`) and task #4725 both audited
the prompts dispatch injects and declined the same items. Each declined correctly
*as a prompt edit*. This task asked the different question: can the harness change
so the prompt no longer needs the text?

Three leads became code. Two are clean verdicts, recorded here so a third audit
does not re-derive them.

---

## What changed

**Dependabot bump classification moved into the harness** (#4706 F1, #4725 F6).
`src/dispatch/bump.rs` classifies the bump from the task's own title and
description — a pure function, no network call — and `build_prompt` renders the
runbook around the verdict. See `AReviewRunbookCarriesOnlyTheBranchThatApplies`
in `docs/specs/dispatch.allium`.

The audit proposed doing this in `scripts/fetch-dependabot.sh` and flagged the
feed-script scope boundary as the reason it stopped. Rust was chosen instead: the
inputs are already on the task, so nothing needs re-fetching; markdown-table
parsing in `jq` is fragile and only `tests/feed_scripts.rs` could cover it; and a
classification computed in one feed script would not exist for a dependabot task
created any other way.

**The runbook's decision table is gone.** Each dispatch renders one
decision body — merge, changelog-check, summarise-breaking-changes, or
say-it-could-not-be-read — and the merge terminal is omitted entirely on a route
that cannot reach it. That dissolves #4725 F6: the five "jump to step 7"
transitions existed to route between branches the harness now decides. What is
left are two genuine guard failures (dep-only, CI), which any route can hit.

**pr-review's diff-size branch was deleted, not moved** (#4706 F2). The runbook
shelled out to `wc -l` to choose between two review commands. The command it
routes to takes a PR target *and* an effort level of its own, so the diff size
was a proxy for a choice that command already makes.

**`wrap_up` now refuses a review-tagged task** (#4706 F3, #4725 F3). The rule was
a prompt line — stated where it could not be enforced. It is now
`ReviewTasksAreNotWrappedUp` in `docs/specs/mcp-task-tools.allium`, and the
refusal names the retag escape hatch so an agent that took a review task over has
somewhere to put real work. Both runbooks dropped their prohibition line.

Note for anyone re-reading #4706: F3 described the prohibition as appearing three
times in `dependabot.md` and twice in `pr-review.md`. Commit `0bc54d80` (task
#4725) had already cut the repeats, so this task removed one line per file, not
five.

**The PR URL is rendered, not re-derived.** `task.url` is set at feed-insert
time, yet steps 1-2 still asked the agent to extract the number and owner/repo
from the description — which is the 500-character truncation. `gh` takes a URL
wherever it takes a number, and a URL names its own repo, so one rendered line
replaced the extract-and-record step and five `--repo <owner/repo>` spellings.

**The wrap-up gate moved onto the model.** The first cut put the tag check in
`TaskService::validate_wrap_up`, leaving `Task::is_wrappable` answering `true`
for a task the service refused. `Task::wrap_up_block() -> Option<WrapUpBlock>`
now owns the whole question and `is_wrappable()` is defined from it, so a future
surface reading the predicate cannot get an answer the gate contradicts.
`get_task` reports the block too, so the `/wrap-up` skill stops at its Step 2
rather than after a retro and a commit nobody wanted.

### The defect this uncovered

The dependabot runbook's step 4 parsed `Bump <pkg> from <X.Y.Z> to <A.B.C>`.
Sampling the live board (epic 275, `airflow-images`) found **13 of 15 open bot PRs
were Renovate-titled** and matched none of it:

| Title | Old step 4 | Now |
|---|---|---|
| `Bump requests from 2.32.4 to 2.33.0` | minor | minor |
| `fix(deps): update dependency deepdiff to v9` | no match → ask | major |
| `chore(deps): update actions/checkout action to v7` | no match → ask | major |
| `fix(deps): update python (non-major)` | no match → ask | non-major group → ask |

So the agent was an always-ask agent in production. #4706 suspected this
("**This mismatch is worth its own investigation** — if Renovate here is not
configured to emit Dependabot-style titles, step 4 of the runbook never matches")
and was right. Classification is best-effort and says so: the feed truncates a PR
body to 500 characters, so Renovate's update table can arrive cut in half, and
`Unknown` is a routable answer rather than a failure.

---

## Clean verdicts — no change, and why

### F4 — the epic-decomposition carve-out is stated twice

**The prompt is the right place. Confirmed, not merely inherited.**

`spec_first_instruction` and `wrap_up_instruction` both name the carve-out
(implementation, *or* work packages for a decomposition task, is what ends the
task). Emitting it only where it applies needs something that identifies a
decomposition task. Nothing does: `TaskTag` has no such variant, no field marks
one, and `decompose-review` marks its *outputs* (`auto_run_plan`) rather than the
task that runs it. `docs/specs/dispatch.allium` already records this as the
reason, and it still holds.

Adding a tag purely to drop one clause from one prompt would put a routing key in
the domain model to save a sentence. The duplication is deliberate and the two
copies qualify different things — above, how the task may *finish*; below, when to
*call* the skill.

### F6 — stable instructions sit last in the assembled prompt

**Still not actionable. Dispatch has no cache breakpoint to move.**

`render_task_prompt` puts the volatile task block and injected knowledge before
the invariant trailing block, which is the reverse of cache-friendly ordering.
But dispatch writes the whole prompt to `.claude-prompt` and launches
`claude … -- "$prompt"` (`src/dispatch/agents.rs::prompt_launch_command`). It
hands the CLI one positional operand. Claude Code owns every caching decision
downstream of that, and dispatch has no flag, no breakpoint, and no request
builder to reorder. Re-ordering the prompt would change what the agent reads for
no measurable gain.

This becomes actionable only if dispatch ever calls the API directly. It does
not: there is no SDK, no HTTP client, and no API key anywhere in the repo.

---

## Left undone, deliberately

**The 500-character body truncation is ours, and widening it is the deeper
fix.** `scripts/fetch-dependabot.sh` does `.[0:500]` on the PR body, so
Renovate's update table — which states the semver kind outright — can arrive cut
in half. The classifier documents that constraint rather than removing it, and
the honest general fix is upstream: widen the slice, or have the feed declare the
kind in `FeedItem` the way `url_type` is already declared for exactly this reason
("inference cannot reach" some values).

Not done here because the blast radius is wrong for this task. `description`
becomes the card's text in the TUI and is injected into every prompt, so widening
it changes every feed's cards, not just this one; and declaring a kind is a
`FeedItem` schema change. Filed as task #4728.

Worth noting the cost is currently small: of the 15 live bot PRs sampled, zero
were the dotted single-package Renovate form that needs the table. This
deployment has `separateMinorPatch` off and groups its non-majors, so single
packages arrive as bare majors (`to v9`) which the title settles on its own.

**`/code-review` is a harness built-in, not a skill dispatch ships.** Nothing
asserts the name resolves, and there is no fallback now that the second command
is gone. The same was true of `/review` and `/review-pr` before, so this is
pre-existing rather than a regression — but dispatch cannot test a command it
does not own, so it stays a known single point of failure.

---

## Still not worth revisiting

Both prior audits agreed and the reasons are durable: `tdd_instruction`
duplicating CLAUDE.md (functioning redundancy), `spec_first_instruction`'s
numbered steps (an order CLAUDE.md mandates), dependabot's "Do not re-check the
PR author" (carries its reason, and `tests/feed_scripts.rs` pins it), and the
worktree-cwd line (a live demonstrated failure with a spec rule behind it).
