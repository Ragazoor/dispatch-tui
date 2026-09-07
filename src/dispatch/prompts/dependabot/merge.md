AUTO-APPROVE + MERGE:
   - gh pr review <PR> --approve --body "Auto-approved by dispatch dependabot agent: <the Bump line above>, CI green, dependency files only."
   - gh pr merge <PR> --squash --auto
   - Note: --auto requires the repo to have branch protection with required checks; without it, the PR merges immediately.
   - Done. Nothing further is needed: the task is auto-cleaned on merge.
