AUTO-APPROVE + MERGE:
   - First check Kognic's GitHub App verdict: gh pr view <PR> --json reviews,comments. The app is `kognic-github-app`. It is safe only if the app's latest review is APPROVED (a new push dismisses its earlier review). If that review is dismissed, absent or anything else, go to ASK THE USER and quote the reason from the app's latest comment, or say the app has given no verdict.
   - gh pr review <PR> --approve, with a body saying what you actually checked.
   - gh pr merge <PR> --squash --auto
   - Note: --auto requires the repo to have branch protection with required checks; without it, the PR merges immediately.
   - Done. Nothing further is needed: the task is auto-cleaned on merge.
