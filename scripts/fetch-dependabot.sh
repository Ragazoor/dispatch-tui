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
#   [{"external_id":"dep:owner/repo#42","title":"#42 Bump foo","description":"...","url":"...","status":"backlog","tag":"pr-review"}]

# ---------------------------------------------------------------------------
# Configure your repositories here (space-separated "owner/repo" slugs).
REPOS="annotell/scala-common annotell/gha-scala annotell/gha-database-bootstrapper annotell/airflow-dags annotell/annotell-data-warehouse annotell/bigquery-export"
# ---------------------------------------------------------------------------

if [ -z "$REPOS" ]; then
  echo "[]"
  exit 0
fi

result="[]"

for repo in $REPOS; do
  # Probe repo existence/auth first — `gh pr list --author app/dependabot`
  # silently returns [] on 404/SSO failures, so we'd never see auth issues.
  probe=$(gh api "/repos/$repo" --jq '.full_name' 2>&1)
  status=$?
  if [ $status -ne 0 ]; then
    echo "fetch-dependabot: $repo — repo unreachable (exit $status): $probe" >&2
    continue
  fi

  raw=$(gh pr list \
    --repo "$repo" \
    --author app/dependabot \
    --state open \
    --json number,title,body,url 2>&1)
  status=$?
  if [ $status -ne 0 ]; then
    echo "fetch-dependabot: $repo — gh pr list failed (exit $status): $raw" >&2
    continue
  fi

  items=$(printf '%s' "$raw" | jq --arg repo "$repo" '[.[] | {
      external_id: ("dep:" + $repo + "#" + (.number | tostring)),
      title: ("#" + (.number | tostring) + " " + .title),
      description: ((.body // "") | .[0:500]),
      url: .url,
      status: "backlog",
      tag: "pr-review",
      labels: [($repo | split("/") | last)]
    }]') || {
    echo "fetch-dependabot: $repo — jq failed on output: $raw" >&2
    continue
  }

  result=$(printf '%s\n%s' "$result" "$items" | jq -s 'add // []')
done

echo "$result"
