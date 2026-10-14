# The PR path

Do all of this after the commit (Step 6) and before the closing sequence (Step 7), then close with `action="pr"`. `wrap_up` runs the verify command, but only after the PR exists, so first run the `Verify command:` line from `get_task` (if it has one) in your worktree and fix any failure before you push. You are writing a real PR whose title and body reflect the actual work; dispatch does not author it.

### Inspect what changed

```bash
git log {base_branch}..HEAD --oneline
git diff {base_branch}...HEAD --stat
git diff {base_branch}...HEAD
```

Read the output. Build a mental model of what shipped: which files changed and why, which behaviours were added/removed/fixed, what the user-visible effect is. If the diff is large, focus on the changes that matter for review (skip generated files, snapshot updates, formatting churn).

### Draft the title and body

**Title** — imperative mood, ≤72 characters, describes the change as a single action. Examples:
- `fix(auth): handle expired refresh tokens without 500ing`
- `feat(tui): add project filter to archive view`
- `refactor(db): split TaskPatch builder into smaller methods`

Avoid `wip:`, `task #N:`, or anything that just restates the task title. The title should be useful in `git log --oneline`.

**Body** — Markdown. There is no template: shape the body to the change. Nothing reads our PR bodies programmatically, so no heading or label is required. Write for a reviewer who has a minute, and apply these principles:

1. **Open with the situation and the consequence**, in two plain sentences: why this PR exists and what goes wrong without it. No heading above them.
2. **Show the change instead of describing it.** If the change has a shape, put the shape on the page: a before/after table for an interface, a few lines of example input and output for a behaviour. The reviewer then sees what changed without reading the diff.
3. **Flag what a reviewer must not miss**, in a few short bullets: a breaking change, something left out on purpose and why, an edge case, what stays the same.
4. **Cut what the diff or CI already shows**: category labels with one bullet each, a restatement of the tests, lists of files or functions, a limit written once in prose and again in a table.
5. **Choose the form from the change.** A bug fix may need two sentences. A tool or API change suits a table plus an example. A migration may need a checklist.
6. **Test it:** could a reviewer parse this in under a minute, and would any line be missed if you deleted it? If not, delete it.

Describe what the change does for its users. For an interface change, the interface (a tool name, a flag, a field) is the user-visible part, so show it. Do not narrate internals the diff already carries.

Two contrasting shapes:

```markdown
Expired refresh tokens made every request return a 500 until the user signed in again.
Now an expired token is refused with a 401 and the client signs in once.
```

```markdown
Agents had to call `list_tasks` three times to build one board view, so a slow store made the board crawl.
`board_view` returns the same data in one call.

| Before | After |
|---|---|
| `list_tasks` + `list_epics` + `list_learnings` | `board_view` |

- Breaking: `list_learnings` no longer accepts `epic_id`.
- Archived tasks are left out on purpose; the board never showed them.
- Existing callers of `list_tasks` are unchanged.
```

Do not add a `## Test plan` section unless the PR is dangerous or breaking (a migration, a change to auth, a change to billing).

Do not reference the dispatch task — no task IDs, no "Implements #N" (GitHub auto-links `#N` to an unrelated issue/PR in this repo).

Notes meant for a reviewing agent rather than a human go in an HTML comment at the end of the body, so they stay out of GitHub's rendered preview.

### Push and create the draft PR

Find the repo slug from the remote:

```bash
git remote get-url origin
```

The slug is the `owner/repo` portion (e.g. `git@github.com:Acme/dispatch.git` → `Acme/dispatch`).

Push the branch:

```bash
git push -u origin {branch}
```

If the push is rejected (non-fast-forward), STOP. Do not force-push without the user's explicit authorisation. Show them the error and ask how to proceed.

Create the PR. Use a HEREDOC for the body so newlines and Markdown survive shell quoting:

```bash
gh pr create --draft \
  --base {base_branch} \
  --head {owner}:{branch} \
  --repo {owner}/{repo} \
  --title "{your authored title}" \
  --body "$(cat <<'EOF'
{your authored body}
EOF
)"
```

`{owner}` is the first part of the repo slug. The `{owner}:{branch}` format is required so `gh` resolves the branch in the same repo as `--repo` (rather than your authenticated user's namespace).

`gh pr create` prints the PR URL on stdout. Capture it — it is the `pr_url` you pass to `exit_session` in Step C.

If `gh` reports `a pull request for branch '...' already exists`, parse the URL it returns and use that — the PR already exists and your job is just to record it.

Then go to Step 8 with `action="pr"` and pass the captured URL as `pr_url`.
