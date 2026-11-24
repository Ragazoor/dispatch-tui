# The PR path

Do this after the commit (Step 6) and before the closing sequence (Step 7), then close with `action="pr"`. Step 6 has already run the verify command; if you changed code since, run the `Verify command:` line from `task_search` again before you push.

Load the `pull-request` skill (`dispatch:pull-request`, or `context_read` with `skill://pull-request/SKILL.md`) and follow it: check what the branch holds, write the title and body from your own diff, push, and create the PR as a draft against `{base_branch}` (pass it as `--base`). Dispatch neither writes nor pushes it for you.

The create command prints the PR's URL, or names the PR that already exists for the branch. Keep that URL: it is the `pr_url` you pass to `session_exit` in the closing sequence (Step 7) with `action="pr"`.
