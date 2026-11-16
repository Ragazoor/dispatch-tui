AUTO-APPROVE + MERGE:
   - gh pr review <PR> --approve, with a body saying what you actually checked.
   - gh pr merge <PR> --squash --auto
   - Note: --auto requires the repo to have branch protection with required checks; without it, the PR merges immediately.
   - Done. Nothing further is needed: the task is auto-cleaned on merge.
