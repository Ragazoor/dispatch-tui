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

# ---------------------------------------------------------------------------
# Repositories: edit repos.conf in the same directory (SSOT), or set REPOS
# directly below as a fallback when repos.conf is not present.
REPOS=()

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
if [[ -f "$SCRIPT_DIR/repos.conf" ]]; then
  # shellcheck source=repos.conf
  source "$SCRIPT_DIR/repos.conf"
fi
# ---------------------------------------------------------------------------

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

# Run "$@" with each authenticated token in turn until one succeeds, printing
# that run's stdout. Fails only when every token fails.
with_any_token() {
  local token out
  for token in "${GH_TOKENS[@]}"; do
    if out=$(GH_TOKEN="$token" "$@" 2>/dev/null); then
      printf '%s\n' "$out"
      return 0
    fi
  done
  return 1
}

# The alert's own link to the security update PR Dependabot opened for it. Only
# GraphQL exposes it; the REST alert above does not.
# shellcheck disable=SC2016 # $owner etc. are GraphQL variables, not shell ones
LINK_QUERY='query($owner: String!, $name: String!, $endCursor: String) {
  repository(owner: $owner, name: $name) {
    vulnerabilityAlerts(states: OPEN, first: 100, after: $endCursor) {
      pageInfo { hasNextPage endCursor }
      nodes { number dependabotUpdate { pullRequest { url title state } } } } } }'

PR_LIMIT=200

# Every page is fetched: this feed syncs in Reconcile mode, so a truncated list
# would delete the tasks (and worktrees) of every alert past the cut. A failure
# on any page fails the whole repo rather than emitting a partial list.
#
# Each alert is then linked to an open PR that already fixes it, when one
# exists — see "scripts/fetch-cve.sh: an alert names the open PR that fixes it"
# in docs/specs/feed-scripts.allium. A failed link lookup never drops an alert:
# it only leaves that repo's alerts unlinked.

for repo in "${REPOS[@]}"; do
  if ! pages=$(with_any_token gh api --paginate "/repos/$repo/dependabot/alerts?state=open&per_page=100"); then
    echo "warning: skipping $repo: no account has access" >&2
    echo '[]'
    continue
  fi
  result=$(jq -s 'add // []' <<<"$pages")

  if own=$(with_any_token gh api graphql --paginate -f query="$LINK_QUERY" \
      -f owner="${repo%%/*}" -f name="${repo#*/}"); then
    own=$(jq -s '[.[].data.repository.vulnerabilityAlerts.nodes[]
      | select(.dependabotUpdate.pullRequest.state == "OPEN")
      | {key: (.number | tostring), value: (.dependabotUpdate.pullRequest | {url, title})}]
      | from_entries' <<<"$own")
  else
    echo "warning: $repo: could not read Dependabot update PRs; alerts emitted without them" >&2
    own='{}'
  fi

  if prs=$(with_any_token gh pr list --repo "$repo" --state open --limit "$PR_LIMIT" \
      --json number,url,title,body,isDraft,author); then
    if [[ $(jq length <<<"$prs") -ge $PR_LIMIT ]]; then
      echo "warning: $repo: $PR_LIMIT open PRs listed, the limit; a fix PR past it is not linked" >&2
    fi
  else
    echo "warning: $repo: could not list open PRs; alerts emitted without Renovate links" >&2
    prs='[]'
  fi

  # Through file handles, not --argjson: PR bodies overflow the argument limit.
  echo "$result" | jq --arg repo "$repo" \
      --slurpfile own <(printf '%s' "$own") --slurpfile prs <(printf '%s' "$prs") '
    # Open, non-draft bot PRs, newest first.
    ($prs[0] | map(select(.author.is_bot == true and (.isDraft | not)))
          | sort_by(-.number)) as $bot_prs |
    [.[] |
    ($repo | split("/") | last) as $repo_name |
    (.security_advisory.severity // "low") as $severity |
    (.security_advisory.cve_id
       // .security_advisory.ghsa_id
       // ("#" + (.number | tostring))) as $advisory_id |
    ([.security_advisory.ghsa_id, .security_advisory.cve_id]
       | map(select(. != null and . != ""))) as $ids |
    (.dependency.package.name // "" | ascii_downcase) as $pkg |
    ($own[0][.number | tostring]
       // first($bot_prs[] | select(
            ((.title + "\n" + (.body // "")) as $text
             | any($ids[]; . as $id | $text | contains($id)))))
       // first($bot_prs[] | select($pkg != "" and
            (.title | ascii_downcase | [splits("[\\s,()\\[\\]`]+")] | index($pkg)) != null))
       // null) as $fix_pr |
    {
      external_id: ("cve:\($repo)#" + (.number | tostring)),
      title: "[\($severity | ascii_upcase)] \($repo_name): \($advisory_id)",
      description: (
        (.security_advisory.summary // "") +
        (if (.security_advisory.description // "") != ""
         then "\n\n" + .security_advisory.description
         else ""
         end) +
        (if $fix_pr != null
         then "\n\nFix PR: \($fix_pr.url) (\($fix_pr.title))"
         else ""
         end)
      ),
      url: .html_url,
      status: "backlog",
      tag: "fix",
      wrap_up_mode: (if $fix_pr != null then "done" else "pr" end),
      labels: ([$repo_name] + (if $fix_pr != null then ["fix-pr"] else [] end)),
      sort_order: (
        {critical: 1, high: 2, medium: 3, low: 4}[$severity] // 4
      )
    }]'
done | jq -s '[.[][]]'
