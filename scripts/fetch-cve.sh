#!/usr/bin/env bash
# Fetch open Dependabot vulnerability alerts (CVEs) for a list of GitHub
# repositories and output them as FeedItem JSON for use as a dispatch feed
# epic command.
#
# Requirements: gh (GitHub CLI, authenticated), jq
#
# Usage:
#   dispatch verify-feed scripts/fetch-cve.sh   # validate output
#   # Or configure as feed_command on the CVE epic with feed_interval_secs = 300
#
# Note: when used as a dispatch feed_command, use the absolute path to this
# script. Relative paths only work if dispatch is launched from the project root.
#
# To add/remove repos, edit the REPOS array below.

set -euo pipefail

REPOS=(
    "annotell/scala-common"
    "annotell/gha-scala"
    "annotell/gha-database-bootstrapper"
    "annotell/airflow-dags"
    "annotell/annotell-data-warehouse"
    "annotell/bigquery-export"
)

if [[ ${#REPOS[@]} -eq 0 ]]; then
  echo '[]'
  exit 0
fi

# Collect tokens for all authenticated github.com accounts so each repo can be
# tried with each key until one succeeds (avoids switching the global account).
GH_TOKENS=()
while IFS= read -r login; do
  token=$(gh auth token --user "$login" 2>/dev/null) && GH_TOKENS+=("$token")
done < <(gh auth status --json hosts 2>/dev/null \
  | jq -r '.hosts["github.com"][].login')

if [[ ${#GH_TOKENS[@]} -eq 0 ]]; then
  echo "error: no authenticated github.com accounts found — run gh auth login" >&2
  echo '[]'
  exit 1
fi

# NOTE: per_page=100 is the GitHub REST API maximum per request. Repos with
# more than 100 open Dependabot alerts will be silently truncated. For full
# pagination use:
#   gh api --paginate "/repos/$repo/dependabot/alerts?state=open&per_page=100"

for repo in "${REPOS[@]}"; do
  result=""
  for token in "${GH_TOKENS[@]}"; do
    if result=$(GH_TOKEN="$token" gh api "/repos/$repo/dependabot/alerts?state=open&per_page=100" 2>/dev/null); then
      break
    fi
    result=""
  done
  if [[ -z "$result" ]]; then
    echo "warning: skipping $repo: no account has access" >&2
    echo '[]'
    continue
  fi
  echo "$result" | jq --arg repo "$repo" '[.[] | {
      external_id: ("cve:\($repo)#" + (.number | tostring)),
      title: (
        "[" + (.security_advisory.severity | ascii_upcase) + "] " +
        (if .security_advisory.cve_id != null
         then .security_advisory.cve_id + ": "
         else ""
         end) +
        .security_advisory.summary
      ),
      description: (.security_advisory.description // ""),
      url: .html_url,
      status: "backlog",
      tag: "fix"
    }]'
done | jq -s '[.[][]]'
