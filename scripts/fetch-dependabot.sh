#!/usr/bin/env bash
# fetch-dependabot.sh — outputs open Dependabot PRs as a FeedItem JSON array
# for use as a dispatch feed_command.
#
# Prerequisites: gh CLI (https://cli.github.com/) and jq must be in PATH.
#
# Usage:
#   1. Set REPOS below to your list of "owner/repo" slugs.
#   2. Set feed_command on your Dependabot epic to the path of this script.
#      Example: /home/you/scripts/fetch-dependabot.sh
#
# Output format (FeedItem):
#   [{"external_id":"dep:owner/repo#42","title":"#42 Bump foo","description":"...","url":"...","status":"backlog"}]

# ---------------------------------------------------------------------------
# Configure your repositories here (space-separated "owner/repo" slugs):
REPOS="annotell/airflow-dags annotell/scala-common"
# Example:
# REPOS="myorg/frontend myorg/backend myorg/infra"
# ---------------------------------------------------------------------------

if [ -z "$REPOS" ]; then
  echo "[]"
  exit 0
fi

result="[]"

for repo in $REPOS; do
  items=$(gh pr list \
    --repo "$repo" \
    --author app/dependabot \
    --state open \
    --json number,title,body,url \
    --jq \
    --arg repo "$repo" \
    '[.[] | {
      external_id: ("dep:" + $repo + "#" + (.number | tostring)),
      title: ("#" + (.number | tostring) + " " + .title),
      description: ((.body // "") | .[0:500]),
      url: .url,
      status: "backlog"
    }]') || continue

  result=$(printf '%s\n%s' "$result" "$items" | jq -s 'add // []')
done

echo "$result"
