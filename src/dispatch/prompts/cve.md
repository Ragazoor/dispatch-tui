This is a CVE remediation task. It is a targeted dependency fix, not a feature.
Do not design it spec-first, and do not write a failing test first — neither
matches the shape of this work.

1. Check the description for a `Fix PR:` line. The feed adds one when an open
   PR in this repo already fixes the alert. If there is none, go to step 2.
   If there is one, review and merge that PR instead of writing your own fix:
   - Read the advisory (step 2) and confirm the PR lands on a version in the
     advisory's fixed set (gh pr diff <PR>). The feed may have matched it by
     package name alone, and a higher version is not automatically patched.
   - Confirm a dependency bot opened it and that it changes only dependency
     manifests and lockfiles (gh pr view <PR> --json author,files,headRefOid).
     Note headRefOid: it is the commit you are reviewing.
   - Check CI: gh pr checks <PR>. It must pass.
   - Check Kognic's GitHub App verdict: gh pr view <PR> --json reviews,comments.
     The app is `kognic-github-app`. Its latest review must be APPROVED.
   - If all of these hold: gh pr review <PR> --approve, with a body saying what
     you checked, then
     gh pr merge <PR> --squash --auto --match-head-commit <headRefOid>, so a
     push after your review cannot be merged unseen. Wrap up on the path
     with no git operations. The feed removes this task once the alert closes.
   - If any check fails, tell the user what you found and stop. Do not write a
     second fix that competes with the PR.
2. Read the advisory. The description carries the summary; the task's URL points
   at the alert. Identify the vulnerable package, the affected range, and the
   exact versions the advisory lists as fixed.
3. Find where this repo pins that package — a manifest, a lockfile, or a
   transitive dependency of something it pins.
4. Apply the smallest change that lands on a fixed version. A higher version
   number is not automatically patched: confirm the version you land on is in
   the advisory's fixed set. Fixes are backported to maintenance branches, so a
   newer minor released earlier can still be vulnerable.
5. If no fixed version exists, or reaching one needs source changes beyond the
   pin, stop and tell the user what you found. Do not invent a workaround.
6. Run the repo's verify command and confirm it passes.
7. Add a test only where the fix changed our own code rather than a pinned
   version. The rule above is against test-first design of a version bump, not
   against testing a patch you wrote yourself.
