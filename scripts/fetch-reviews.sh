#!/usr/bin/env bash
# fetch-reviews.sh — outputs open PRs awaiting your review as a FeedItem JSON array,
# for use as a dispatch feed_command.
#
# Prerequisites: gh CLI (https://cli.github.com/) and jq must be in PATH.
#
# Usage:
#   1. Copy this file to scripts/local/fetch-reviews.sh
#   2. Set ORGS below to your list of GitHub organisation slugs.
#   3. Set feed_command on your Reviews epic to the absolute path of the local copy.
#
# Output format (FeedItem):
#   [{"external_id":"review:org/repo#42","title":"#42 PR title","description":"...","url":"...","status":"backlog","tag":"pr-review","labels":["repo"]}]
#
# Note: review-requested:@me includes team review requests when you are a member
# of the requested team — no separate per-team query needed.

set -euo pipefail

# ---------------------------------------------------------------------------
# Organisations to search. Fill in your org slugs, e.g.:
#   ORGS=("myorg" "another-org")
ORGS=()
# ---------------------------------------------------------------------------

if [[ ${#ORGS[@]} -eq 0 ]]; then
  echo "[]"
  exit 0
fi

owner_flags=()
for org in "${ORGS[@]}"; do
  owner_flags+=(--owner "$org")
done

raw=$(gh search prs \
  --state=open \
  --review-requested=@me \
  "${owner_flags[@]}" \
  --json number,title,body,url,repository,isDraft,updatedAt \
  --limit 100 2>&1) || {
  echo "fetch-reviews: gh search prs failed: $raw" >&2
  echo "[]"
  exit 0
}

printf '%s' "$raw" | jq '[
  .[] |
  select(.isDraft == false) |
  {
    external_id: ("review:" + .repository.nameWithOwner + "#" + (.number | tostring)),
    title: ("#" + (.number | tostring) + " " + .title),
    description: ((.body // "") | .[0:500]),
    url: .url,
    status: "backlog",
    tag: "pr-review",
    labels: [.repository.name]
  }
]'
