---
name: preference-review
description: >-
  Review a GitHub pull request the way the operator reviews, using their review
  preference files (general, per-language and per-repo "Look for" and "Do not
  raise" lists). Use when a dispatch PR review task runs it, when the user says
  "/preference-review", or asks to review a PR "with my preferences" or "the
  way I review".
---

# Preference review

**Announce at start:** "I'm using the preference-review skill to review this PR."

Review one PR against the operator's preference files. Findings are verified
against the PR's real code, then filtered by taste, then shown worst first.

Never post to GitHub (no comments, reviews, approvals or labels) unless the
user asks in this session. Never edit the PR's code.

## Step 1: Resolve the target

Take the PR number or URL from the argument. With none, take it from the task
description (`get_task` on your task) or the conversation. If there is still
none, ask the user one question.

Get the facts with `gh`, passing `--repo <owner>/<repo>` when you have a URL:

```bash
gh pr view <pr> --json number,title,body,url,author,baseRefName,headRefOid,files
gh pr diff <pr>
```

`owner` and `repo` come from the URL (or `gh repo view --json nameWithOwner`
for a bare number).

**Get the PR's code, not your checkout's.** A dispatch worktree sits on its own
branch, not the PR's. Reviewers and verifiers must read files at
`headRefOid`:

- If the current repo is `<owner>/<repo>`: `git fetch origin pull/<number>/head`,
  then read a file with `git show <headRefOid>:<path>`. Do not check out the
  PR branch in the worktree.
- Otherwise: `gh repo clone <owner>/<repo> <scratch dir>/<repo> -- --filter=blob:none`,
  then fetch and read the same way inside that clone. The scratch dir is your
  session's scratchpad, not `/tmp`.

Read the repo's `CLAUDE.md` at `headRefOid` too, if it has one.

## Step 2: Load preferences

The preference folder is `$DISPATCH_REVIEW_CONTEXT` when that variable is set
and not blank, else `~/.local/share/dispatch/review-context`. Resolve it with
`realpath` and keep the absolute path: step 7 writes there.

From that folder, read whichever of these exist:

- `general.md` — how the operator reviews everywhere.
- `languages/<language>.md` for each language the changed files use. List
  `languages/` and match by file extension (`.py` → `python.md`,
  `.scala`/`.sc`/`.sbt` → `scala.md`, `.sql` → `sql.md`, `.rs` → `rust.md`,
  `.ts`/`.tsx` → `typescript.md`, and so on).
- `repos/<owner>__<repo>.md` — for this repo only (two underscores).

Each file has a **Look for** and a **Do not raise** section. A missing file is
normal. If the folder does not exist, review without preferences: say so in
one line, run steps 3–6 with only the repo's `CLAUDE.md`, and skip step 7.

Tell the user which files you loaded, by path, before you review.

## Step 3: Review

Start three `general-purpose` agents in one message, so they run in parallel.
Do not use reviewer agent types from other plugins: they are not on every host.
Give each one:

- the PR title, body and full diff;
- how to read files at `headRefOid` (step 1);
- the full text of every loaded preference file, inline;
- the repo's `CLAUDE.md`, inline;
- one focus:
  1. **Correctness** — bugs, wrong edge cases, broken error handling, races.
  2. **Design and boundaries** — does the code belong where it is, contracts,
     coupling, scope creep, naming that misleads.
  3. **Tests** — do the tests prove the new behaviour, and would they fail if
     it broke.

Tell each agent: the **Look for** entries are what this operator cares about,
so check every one that applies to the diff. Report only problems the PR
introduces. Return each finding as `file`, `line`, `claim` (one sentence) and
`failure scenario` (concrete input or state → wrong result). No praise, no
summary.

Merge the three lists and drop duplicates.

## Step 4: Verify

Start one `general-purpose` agent for all findings (or one per finding when
there are more than about eight). Give it each finding and the way to read code
at `headRefOid`. Its job is to **disprove** each finding against the real code:
read the lines, follow the callers, check the tests. It returns `confirmed`,
`plausible` or `refuted` with one line of evidence.

Drop `refuted`. Keep `plausible` but mark it. Count what you dropped.

## Step 5: Filter

Drop each remaining finding that matches a **Do not raise** entry in a loaded
file. A more specific file wins: a repo file's **Look for** keeps a finding that
`general.md`'s **Do not raise** would drop. Count what you dropped, and keep a
one-line note of which entry dropped each, in case the user asks.

## Step 6: Present

Show a short list, worst first. For each finding:

- `path:line` and a severity (blocking, should fix, nit);
- the comment, written as the operator would write it on the PR: short and
  direct, a question when the point is a question ("is this used?"), no
  hedging and no praise. Follow any voice notes in `general.md`;
- the failure scenario in one line;
- `(plausible)` when step 4 could not confirm it.

End with one line of counts: "Dropped N unconfirmed (step 4) and M by Do not
raise (step 5)." Then stop and wait for the user.

## Step 7: Learn

After the user reacts, propose preference edits. Skip this step if no
preference folder exists.

- A finding the user rejects becomes a proposed **Do not raise** entry.
- A point the user raises that the review missed becomes a proposed **Look
  for** entry.

Put each in the most specific file that fits: `repos/<owner>__<repo>.md`, then
`languages/<language>.md`, then `general.md`. Create a missing file with both
section headings. Match the existing entries' style: a bold lead and one or two
sentences.

Show the user each edit: the absolute file path (resolved in step 2) and the
exact lines to add. Write it only after the user approves. That folder is
outside your worktree; this approved edit is the only write there this skill
allows. Do not commit in the folder; it belongs to the operator.
