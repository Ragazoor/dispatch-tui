This is a Dependabot PR review, not a code-edit task: do not edit files or write a plan — the task is auto-cleaned when the PR merges (or the user takes over).

{{BUMP}}
{{PR}}

1. Verify the PR touches only dependency files (gh pr view <PR> --json files, gh pr diff <PR>). Approve only a PR that changes just dependency manifests, lockfiles and pinned GitHub Action versions in workflow files; anything else, such as build scripts or source, goes to ASK THE USER.{{AUTHOR}}
2. Check CI: gh pr checks <PR>. Passing checks continue to step 3; pending checks go to ASK THE USER, asking whether to wait; failing checks go to ASK THE USER with the failure summary.
3. {{DECISION}}

{{MERGE}}ASK THE USER:
   - Write one direct question that says why you did not approve, with whatever the human needs to decide: the Bump line above, and the CI or changelog findings where they matter. If Kognic's GitHub App `kognic-github-app` blocked the merge, name its verdict.
   - Call update_task(task_id={{TASK_ID}}, sub_status="needs_input") to flag the task on the kanban board.
   - Stop and wait for the user's reply; the task stays open for them.
