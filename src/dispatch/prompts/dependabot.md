This is a Dependabot PR review, not a code-edit task: do not edit files or write a plan — the task is auto-cleaned when the PR merges (or the user takes over).

{{BUMP}}
{{PR}}

1. Verify the PR touches only dependency files:
   - Run: gh pr view <PR> --json files
   - Run: gh pr diff <PR>
   - Every changed file path must match one of: Cargo.toml, Cargo.lock, package.json, package-lock.json, pnpm-lock.yaml, yarn.lock, requirements*.txt, pyproject.toml, uv.lock, go.mod, go.sum, Gemfile, Gemfile.lock, composer.json, composer.lock, .github/workflows/*.
   - If that check fails, go to ASK THE USER.
   - Do not re-check the PR author. The feed that created this task lists PRs by bot author, so a task only exists for a PR that already passed that filter; re-deriving it costs a call and can only ever agree.
2. Check CI: gh pr checks <PR>.
   - All checks passing -> continue to step 3.
   - Any check pending -> go to ASK THE USER and ask whether to wait.
   - Any check failing -> go to ASK THE USER with the failure summary.
3. {{DECISION}}

{{MERGE}}ASK THE USER:
   - Write ONE direct question that includes: the Bump line above, the dep-only verdict, the CI status summary, the changelog summary or its absence, and the specific reason you are not auto-merging.
   - Call update_task(task_id={{TASK_ID}}, sub_status="needs_input") to flag the task on the kanban board.
   - Stop and wait for the user's reply; the task stays open for them.
