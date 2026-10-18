#!/usr/bin/env bash
# check-commit-msg.sh — reject a commit message that is not a Conventional Commit.
#
# Usage: scripts/check-commit-msg.sh <commit-message-file>
#
# release-please (release-please.yml) reads the commit subjects on main to pick
# the next version and write the changelog, so a subject it cannot parse is a
# change it silently ignores. The subject must be
#
#     <type>[(scope)][!]: <description>
#
# with <type> one of feat, fix, docs, chore, refactor, test, perf, ci, build,
# style, revert. A breaking change is `!` after the type or scope, or a
# `BREAKING CHANGE:` footer.
#
# Exempt: merge commits ("Merge ..."), git's own revert subject ('Revert "..."')
# and autosquash markers (fixup!, squash!, amend!). None of these is a change
# the changelog should list.
#
# Run by .githooks/commit-msg; the self-test is scripts/test-check-commit-msg.sh.
set -euo pipefail

msg_file="${1:?usage: check-commit-msg.sh <commit-message-file>}"

# The subject is the first line that is not a comment (git strips `#` lines).
subject="$(grep -v '^#' "$msg_file" | sed -n '1p')"

types='feat|fix|docs|chore|refactor|test|perf|ci|build|style|revert'

case "$subject" in
    'Merge '* | 'Revert "'* | 'fixup! '* | 'squash! '* | 'amend! '*) exit 0 ;;
esac

if [[ "$subject" =~ ^($types)(\([^\)[:space:]][^\)]*\))?!?:\ [^[:space:]] ]]; then
    exit 0
fi

cat >&2 <<MSG
Commit message rejected: the subject is not a Conventional Commit.

  subject: ${subject:-<empty>}

Expected:  <type>[(scope)][!]: <description>
Types:     ${types//|/, }
Examples:  feat(tui): show agent names in the tree
           fix: stop the feed poll leaking a connection
           refactor!: rename the task alias

See https://www.conventionalcommits.org/ (docs/conventions.md, "Conventional commits").
MSG
exit 1
