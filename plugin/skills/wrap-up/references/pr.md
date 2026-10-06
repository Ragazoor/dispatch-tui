# The PR path

Do all of this after verification (Step 7) and before the closing sequence (Step 8), then close with `action="pr"`: verify before you push and open the PR, not after. You are writing a real PR whose title and body reflect the actual work; dispatch does not author it.

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

**Body** — Markdown, `## Summary` grouping the changes under bold category labels — the same structure CodeRabbit's auto-generated PR summaries use. Only include the labels that apply:

- **Breaking Changes**
- **New Features**
- **Bug Fixes**
- **Refactor**
- **Performance**
- **Documentation**
- **Tests**
- **Chores**

```markdown
## Summary

- **Bug Fixes**
  - {user-visible change and why it matters, in plain language}
- **Documentation**
  - {user-visible change and why it matters}
```

Each bullet describes the user-visible change and why — never a function, class, file, or variable name. Skip a category entirely if nothing in the PR belongs to it; a one-line fix can have a single bullet under a single category — don't pad it out. If a PR breaks a public API or requires a migration, put that under **Breaking Changes** first, regardless of what else changed.

Do not add a `## Test plan` section unless the PR is dangerous or breaking (a migration, a change to auth, a change to billing) — omit it by default.

Do not reference the dispatch task — no task IDs, no "Implements #N" (GitHub auto-links `#N` to an unrelated issue/PR in this repo).

If the change has UI implications, add screenshots or a description of the visual effect under a `## Notes` section.

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
