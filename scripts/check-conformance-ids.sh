#!/usr/bin/env bash
# Reject a conformance test that asserts the real store and the in-memory twin
# hand out the same generated id.
#
# The real store's auto-inc ids jump (a fresh block of 4096 after a restart or
# reallocation) while the twin's count 1, 2, 3, so such an assertion fails at
# random under load. It failed `main`'s verify on d00aab72 (task ids,
# `left: TaskId(4096)`, `right: TaskId(2)`); e6253a0a had already fixed the same
# class for work-package ids. Create through `create_task_both` /
# `create_epic_both` (tests/memory_caller_conformance.rs), or call the twin's
# `align_next_*_id` with the real id before replaying a create, and never compare
# the two ids.
#
# What it flags: an `assert_eq!` / `assert_ne!` statement (up to its `;`) in
# tests/memory_caller_conformance.rs or tests/memory_caller_conformance/*.rs
# whose message says `same ... id`, `generated ... id` or `hand out`. An
# assertion that needs one anyway carries `// allow-cross-store-id: <why>`
# inside the statement.
#
# Limits, accepted: a grep over the message text, so an id comparison with no
# such message, or one worded otherwise, is not seen. Review catches those.
#
# Run from the repo root. Exits 1 and lists each offending line.
set -euo pipefail

files=(tests/memory_caller_conformance.rs)
while IFS= read -r f; do files+=("$f"); done < <(find tests/memory_caller_conformance -name '*.rs' 2>/dev/null | sort)

found=0
for f in "${files[@]}"; do
    [[ -f "$f" ]] || continue
    hits="$(perl -0777 -ne '
        while (/(assert_(?:eq|ne)!\s*\((?:[^;]*?))(;)/sg) {
            my ($stmt, $start) = ($1, $-[1]);
            next if $stmt =~ /allow-cross-store-id:/;
            next unless $stmt =~ /"[^"]*(?:\bsame\b[^"]*\bid\b|\bgenerated\b[^"]*\bid\b|hand out)[^"]*"/i;
            my $line = 1 + (substr($_, 0, $start) =~ tr/\n//);
            print "$line\n";
        }' "$f")"
    if [[ -n "$hits" ]]; then
        while IFS= read -r line; do
            echo "$f:$line: asserts the real store and the twin hand out the same generated id" >&2
        done <<<"$hits"
        found=1
    fi
done

if [[ "$found" == 1 ]]; then
    echo "The real store's ids jump; align the twin to the real id instead (see this script's header)." >&2
    exit 1
fi
