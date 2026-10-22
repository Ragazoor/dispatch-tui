The description's `Fix PR:` line names an open PR in this repo that already
fixes the alert. Review and merge that PR instead of writing your own fix.

1. Read the advisory. The description carries the summary; the task's URL points
   at the alert. Identify the vulnerable package and the exact versions the
   advisory lists as fixed.
2. Confirm the PR lands on a version in the advisory's fixed set
   (gh pr diff <PR>). The feed may have matched it by package name alone, and a
   higher version is not automatically patched: fixes are backported to
   maintenance branches, so a newer minor released earlier can still be
   vulnerable.
3. Confirm a dependency bot opened it and that it changes only dependency
   manifests and lockfiles (gh pr view <PR> --json author,files,headRefOid).
   Note headRefOid: it is the commit you are reviewing.
4. Check CI: gh pr checks <PR>. It must pass.
5. Check Kognic's GitHub App verdict: gh pr view <PR> --json reviews,comments.
   The app is `kognic-github-app`. Its latest review must be APPROVED.
6. If all of these hold: gh pr review <PR> --approve, with a body saying what
   you checked, then
   gh pr merge <PR> --squash --auto --match-head-commit <headRefOid>, so a push
   after your review cannot be merged unseen. Wrap up on the path with
   no git operations. The feed removes this task once the alert closes.
7. If any check fails, tell the user what you found and stop. Do not write a
   second fix that competes with the PR.
