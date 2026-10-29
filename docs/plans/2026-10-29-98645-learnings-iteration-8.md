# Learnings iteration 8: a verdict for "read, did not fit"

Task #98645, run 8, 2026-10-29. Follows `2026-10-28-98511-learnings-iteration-7.md`.

One change, spec-first, with the operator's approval in this session:

- `rate_learning` takes a third verdict, `not_applicable`: the agent read an injected entry
  and it did not fit the task. Weight 0. A first one stores the verdict and does not write
  the learning row (not even `updated_at`, so it cannot keep a stale entry alive). A later
  `helped` or `wrong` from the same task replaces it, as with any verdict.
- Spec: `core.allium` (`VerdictKind`) and `learnings.allium` (`RateLearningViaMcp`, the
  Retrievals & Verdicts rules, the injection header, the wrap-up nudge). The rule for
  `updated_at` now reads "when the score moves", which is the same as before for helped and
  wrong.
- Code: `verdict_weight` in the module (`not_applicable` → 0), `LearningVerdict` on the
  board, the memory twin now uses `as_str()`, the tool schema and its description, the
  injection header, the wrap-up PR nudge. `module.wasm` rebuilt. No table, bindings or
  rules-version change: the column is already an opaque string, and accepting one more
  value moves no rule into the module (`RULES_VERSION`'s own criteria).
- Skills: `learnings` and `wrap-up` ask for `not_applicable` on injected entries that did
  not fit; `docs/how-to.md` too.
- `scripts/learnings-metrics.py`: a new section, injected (task, learning) pairs by the
  task's standing verdict per week: helped / wrong / not_applicable / unrated.

## Why this, why now

Run 7 set the condition: if the rated-only unhelped share still sits near 72%, add the
verdict. It reads 71% (W44, 38/53), after 72% in W43. The "unhelped" bucket mixes two
things the store cannot tell apart: "read, did not fit" (a precision problem: rescope or
prune the entry) and "never rated" (a prompt problem). Seven runs have reasoned about that
bucket without a signal. This is the outcome signal the baseline report asked for first.

What it measures, honestly: relevance of injected entries, not prevented mistakes. It
says which entries are noise in which tasks. It does not say an agent avoided an error.

History the operator saw before approving: an `unused` verdict ("retrieved but did not
apply") existed until 026e619e (2026-05-27), which dropped it when wrap-up verdicts became
`rate_learning`. No reason or data was recorded beyond collapsing the two paths. The new
verdict differs in being asked for only on injected entries.

Web check: recent work treats usefulness, not similarity, as the retrieval bottleneck
([arXiv 2603.07670](https://arxiv.org/pdf/2603.07670)), and uses an explicit agent or user
feedback score as a re-ranking term with 0 for "no feedback"
([MARK, arXiv 2505.05177](https://arxiv.org/pdf/2505.05177)). Both support collecting an
explicit "did not fit" before using it to rank. Only abstracts and summaries read.

## Measured before the change (2026-10-29, W44 partial)

Injected pairs by standing verdict, all scopes. `learning_verdicts` only has rows from W43
on, so W44 is the baseline:

| Week | helped | wrong | not_applicable | unrated |
|---|---:|---:|---:|---:|
| W43 | 27/521 (5%) | 1 | n/a | 493 (94%) |
| W44 | 60/373 (16%) | 2 | n/a | 311 (83%) |

Other readings:

- W45 items from run 7 (rated-only share, #565/#500 only in scala/terraform tasks) cannot be
  read yet: W44 is still running.
- PRs with a bullet list after the prose rule: 1 / 2. Too few to read.
- `record_learning` "unknown field `content`" now hits 19 tasks in 14 days (11 in run 4).
  46 refused `record_learning` calls since 10-15; in every case the next call has the right
  fields. One wasted call each, no harm. Runs 3, 5 and 6 declined an alias; that holds.

## Metrics table (run-to-run)

| Measure | Run 7 (after) | Run 8 (before) | Run 8 (after) |
|---|---:|---:|---:|
| Learnings | 668 | 677 | 677 |
| Never upvoted | 328 | 329 | 329 |
| Negative score | 5 | 5 | 5 |
| Retrieval rows | 15,095 | 15,340 | 15,340 |
| Total upvotes | 1,183 | 1,211 | 1,211 |
| Retrieved 20+, never upvoted | 35 | 35 | 35 |
| Unrated and older than 30 days | 170 | 171 | 171 |
| `record_learning` refused, latest week | n/a | 14 / 56 (W44) | 14 / 56 |
| User-scoped unhelped, rated tasks only | 40 / 55 (72%, W44) | 38 / 53 (71%, W44) | read W45 |
| **Injected pairs unrated** | n/a | **311 / 373 (83%, W44)** | read W45 |
| **Injected pairs not_applicable** | n/a | 0 (no verdict) | read W45 |
| PRs with a bullet list, after prose rule | 0 / 0 | 1 / 2 | read later |

## Declined this run

- A score penalty for `not_applicable`. The operator chose signal only: an entry that does
  not fit one repo may fit another, and mixing "off-topic here" with "wrong" would punish
  it everywhere. Collect first, decide on ranking with data.
- An alias for `content` in `record_learning`: see above.
- Pruning the 35 never-upvoted entries: run 7's reasons hold; the new verdict is the
  evidence that pruning lacked.

## Next run should try

1. The board must restart on this commit (and the plugin be reinstalled) before the signal
   flows. Then read `verdict_coverage_metrics` for W45. Success: unrated falls well below
   83% and `not_applicable` takes a real share.
2. If unrated stays near 83%, agents skip rating: look at whether the wrap-up step runs
   at all on done-path tasks, not at the store.
3. If `not_applicable` concentrates on a few entries (the script lists the top ten), show
   that inventory and rescope or delete those entries. That is prune-by-evidence, with
   evidence.
4. Still open from run 7: the W45 rated-only share, #565/#500 placement, #12308 and #734.
