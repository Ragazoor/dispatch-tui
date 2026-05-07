#!/usr/bin/env bash
# Verify every `src/...rs` path mentioned in CLAUDE.md actually exists.
# Run from the repo root. Exits non-zero on the first missing path.
set -euo pipefail

DOC="${1:-CLAUDE.md}"

if [[ ! -f "$DOC" ]]; then
    echo "check-doc-paths: $DOC not found" >&2
    exit 2
fi

missing=0
while IFS= read -r path; do
    if [[ ! -e "$path" ]]; then
        echo "check-doc-paths: missing path referenced in $DOC: $path" >&2
        missing=$((missing + 1))
    fi
done < <(grep -oE 'src/[A-Za-z0-9_/.-]+\.rs' "$DOC" | sort -u)

if (( missing > 0 )); then
    echo "check-doc-paths: $missing missing path(s) in $DOC" >&2
    exit 1
fi

echo "check-doc-paths: all paths in $DOC exist"
