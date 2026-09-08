# 4728 — Should the feed declare a bump's kind, or the harness keep reading it?

Follow-up to #4727 (`docs/plans/4727-harness-vs-prompt-verdicts.md`), raised by that
task's altitude review. The proposal was that `src/dispatch/bump.rs` compensates for
a constraint this repo owns — the 500-character body slice in the feed scripts — and
that a `FeedItem` field declaring the kind would remove the guessing at source.

Both halves of that proposal rest on claims about the data. Neither claim held.
The verdict is recorded here so a third pass does not re-derive it.

---

## What the live board actually carries

Sampled the 15 open bot PRs behind epic 275 (`annotell/airflow-images`) — the same
queue #4727 sampled — and sliced each body at 500 characters the way the feed does.

| | present within the 500-char slice |
|---|---|
| A version pair (`` `==8.6.2` → `==9.1.0` ``) | 13 of 13 Renovate bodies |
| A `\| major \|` kind cell | 1 of 15 (`actions/checkout`) |
| A GitHub label naming the semver kind | 0 of 15 |

Three things follow.

**Renovate does not label the kind.** Every sampled PR carries exactly
`dependencies, autoreview`. There is no label to pass through, so the "add `labels`
to the `--json` list" route in the proposal has nothing to fetch.

**The truncation is not costing anything.** The table header and its first row fit
inside the slice with room to spare; what the slice removes is the release notes
below the table, which no branch reads. The two Dependabot PRs state their pair in
the title, which is never truncated. So neither widening the slice nor moving the
cap to render time buys a single card.

**The kind cell was never the right thing to read.** `TABLE_CELL_RE` matches
`| Package | Type | Update | Change |` — the shape Renovate emits for the **action**
datasource. A package update emits `| Package | Change | Age | Confidence |`, which
has no Update column at all. The cell is absent by construction there, not by
truncation. That is why it hit 1 of 15.

## Verdict 1 — the feed does not declare the kind

`FeedItem.url_type` is the precedent for a declared field, and it earns its place
because the **script** knows something inference cannot reach: it fetched from the
Dependabot alerts endpoint, so it knows the URL is a security alert. Nothing
equivalent holds for the bump kind.

With no semver label to read, a declaring script would have to parse the same
markdown body in `jq` — which is what #4727 rejected — and it would have to do it in
**both** shipped producers (see verdict 3) — and both are user-owned copies, so
neither copy that actually runs would receive the fix. `fetch-reviews.sh` is a
template the user copies to `scripts/local/` and edits; `fetch-dependabot.sh` is
installed into `<data_dir>/scripts/` by setup and then never overwritten, which is
the stronger case of the two. The declared field would also not reach a hand-created
dependabot task, which the harness classifier does.

Declaring would move the guessing somewhere less testable and less reachable, and
buy a `FeedItem` field plus a `Task` column and a migration to do it. Rejected.

## Verdict 2 — the classifier reads the Change cell instead

One new rule in `bump.rs`: for a Renovate single-package title, read the version
pair out of the update table's Change cell and compare it as semver. The full
reading order is now

1. the Dependabot title form,
2. Renovate's grouped form,
3. **the body's Change-cell version pair** (new),
4. the Renovate title's bare major target (`to v9`),
5. the Dependabot sentence in the body,
6. a kind cell anywhere in the body,
7. unclassified.

Steps 3 and 4 are gated on a Renovate title; 5 and 6 are not. The gate on the pair
is deliberate — a pair says which versions moved, not that this task is a dependency
bump at all — while the kind cell stays the title-agnostic last resort it always was.

The pair is read **before** the bare target even though both yield the same kind for
a title that carries one, because only the pair names the source version. The Bump
line the prompt renders goes from `Bump: major — deepdiff → v9` to
`Bump: major — deepdiff 8.6.2 → 9.1.0`.

Two guards make the rule safe:

- **Exactly one pair is required.** A pair belongs to one row, so reporting it for a
  multi-row body would attribute one package's versions to the whole PR. This is
  stricter than the kind-cell rule, which refuses only rows that *disagree*.
- **Only table rows are scanned.** Release notes are most of what a body holds and
  they quote versions freely.

Neither guard is sufficient on its own, and the measurement showed why: the slice
cuts a group's table after its first row, so **a truncated group body presents
exactly one pair and reads as single-package**. PR #79 (`update python
(non-major)`) is exactly that. The grouped-**title** branch running first is what
keeps it out of the changelog branch — load-bearing ordering, not tidiness.

Re-running the new precedence over all 15 sampled PRs: every one classifies, every
kind agrees with what the old rules said, and 13 gain a source version.

## Verdict 3 — the Dependabot half is live, not fallback-only

The review claimed `fetch-dependabot.sh:47` filtering `--author app/kognic-renovate`
left the Dependabot branch of the classifier with no in-repo producer. It has one.

`fetch-reviews.sh::to_feed_items` tags **any** PR whose author login ends in `[bot]`
as `dependabot`, and its bot-author pass reads `bots.conf`. PRs #25 and #29 are
authored by `app/dependabot`, carry `Bump <pkg> from <X> to <Y>` titles, and are on
the board right now. Every one of the 15 sampled tasks has an external_id prefixed
`review:` — so `fetch-reviews.sh` is in fact the *only* live producer of the
dependabot queue in this deployment, and `fetch-dependabot.sh` is not wired to
anything.

That still leaves the template wrong for a user who does wire it. `aff2e3fb` dropped
`app/dependabot` from it as part of a Renovate migration that has not finished, and a
bot login is deployment-specific anyway. `fetch-dependabot.sh` now runs one
`gh pr list --author "$author"` pass per entry in `bots.conf`'s `BOT_AUTHORS` — the
same list `fetch-reviews.sh` reads, so a deployment spells its bots once. An absent
or empty `BOT_AUTHORS` falls back to `app/kognic-renovate`, so an existing copy keeps
emitting what it emitted before, and `dispatch setup` now installs `bots.conf`
alongside `repos.conf`.

## What the weed pass caught

Running `allium weed` over the changed area found seven divergences, five of them
introduced by the first cut of this task's spec text. Worth noting because four of
the five were the same failure mode: **compressing a list makes the omitted case
look like it does not exist.**

- The rewritten reading order listed five steps and ended "anything else is
  unclassified", which silently deleted the Dependabot body-sentence step the code
  still has.
- It scoped all three trailing steps to "a Renovate single-package title", which
  narrowed the kind cell — a title-agnostic fallback in the code — to something it is
  not.
- The `feeds.allium` two-producer paragraph said "this script's per-PR emission, and
  `to_feed_items` above", but sits inside the `fetch-reviews.sh` section, so both
  halves resolved to the same script.
- The declared-field rejection blamed the copied-template hazard on
  `fetch-reviews.sh` alone. `fetch-dependabot.sh` is the stronger case: setup
  installs it into `<data_dir>/scripts/` and never overwrites it.
- `bots.conf` still described itself as `fetch-reviews.sh`'s file, and the empty-list
  behaviour now differs between its two readers (one skips its pass, the other falls
  back to `app/kognic-renovate`). That is the shipped state of the file, so the file
  now says both, and `tests/feed_scripts.rs` pins it.

## Verdict 4 — a review-tagged feed item must name its PR

The seventh weed finding was the runbook telling the agent "the feed that created
this task lists PRs by bot author, so a task only exists for a PR that already passed
that filter". False on a hand-created dependabot task, which the same function
explicitly supports (it renders "PR: not recorded" for it). Pre-existing, not
introduced here — but the fix has two halves, and each is only sound with the other.

**The feed boundary now enforces it.** `AReviewTaggedFeedItemNamesItsPr`: a
`FeedItem` whose tag satisfies `TaskTag::is_review` — `dependabot` or `pr_review` —
must carry a url that resolves to `pr`. An empty url, or one typing as issue,
security_alert or other, **rejects the whole emission**, exactly as an unknown `tag`
does and for the same reason: both are producer bugs a silent per-item drop would
hide. Blast radius is accepted and stated — `fetch-reviews.sh` emits one array for the
whole review board, so one bad item costs the cycle rather than one card.

Both shipped producers already satisfy it by construction: each emits `url` straight
from `gh`, always a `/pull/` URL, and `UrlType::infer` types anything containing
`/pull/` as `pr`. So the rule costs nothing today and pins what the runbook is
entitled to assume.

It lives in `parse_feed_items` rather than beside the strict `tag` rule in
`Deserialize`, because serde's field attributes cannot see a sibling field and a
shadow struct would duplicate all ten fields — the parity-drift shape this repo
guards against elsewhere. `parse_feed_items` is the single decode point for all three
entry points, so a check there is just as uniform.

A side effect worth noting: the url/url_type precedence existed in two places, and
`FeedItem::resolved_url_type` is now the only one. The upsert query and the validator
read the same answer, so the type a rejection fires on is exactly the type the row
would have got.

**The prompt requires both halves.** The author bullet moved into its own fragment
and renders only when the task carries an `external_id` **and** names a pr-typed url.
Neither alone is sufficient, and each failure is a real route: the CVE feed sets
`external_id` too and `update_task` retags anything `dependabot` with no `is_review`
guard, while `update_task` will equally put a PR url on a task no feed touched. A
second weed pass caught the first version of this gate reading `external_id` only, and
the spec claiming that test was "exact rather than a proxy" — it was not.

One residual is accepted and now stated: the gate proves a feed created the task and a
PR is named, not that *that* feed filtered by author. A user-authored feed emitting
pr-typed items tagged `dependabot` without an author filter would still be told one
ran. Closing it needs the feed to declare the filtering — a `Signal` — which is a
schema change for a case no shipped script produces. Both shipped PR producers do
filter by author.

Six fixtures across four files built review-tagged items with no url and now fail the
rule. Each was updated rather than exempted; the dependabot prompt snapshot passes
unchanged, which confirms the feed-created path renders exactly as before.

The rule guards the **decode**, not the storage boundary: `upsert_feed_tasks` does not
re-validate, so a struct-literal `FeedItem` reaches the DB unchecked. No production
path does that — the only two decoders both go through `parse_feed_items`, and nothing
outside tests builds the struct — but several DB-layer tests do, and they assert
ingest rather than decode. A second enforcement point there would put a
mid-transaction failure on a path with no unguarded producer, so the boundary stays
where the wire is. The spec now says this instead of claiming the only bypass was a
decode test.


## Where this is recorded

- `AReviewRunbookCarriesOnlyTheBranchThatApplies` in `docs/specs/dispatch.allium` —
  the precedence, both guards, why the kind cell ranks last, the measurement behind
  keeping the slice, and the rejection of the declared field.
- The `fetch-dependabot.sh` bullet and the "TWO shipped producers" paragraph in
  `docs/specs/feeds.allium` — the `bots.conf` filter and why the Dependabot title
  form is live.
