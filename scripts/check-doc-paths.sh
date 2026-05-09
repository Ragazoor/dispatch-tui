#!/usr/bin/env bash
# Verify every `src/...rs` path mentioned in our agent-facing docs actually
# exists. By default scans CLAUDE.md plus the topic files under docs/ that
# CLAUDE.md points at. Pass an explicit path to scan a single file instead.
# Run from the repo root. Exits non-zero on the first missing path.
set -euo pipefail

if [[ $# -gt 0 ]]; then
    DOCS=("$@")
else
    DOCS=(
        CLAUDE.md
        docs/architecture.md
        docs/conventions.md
        docs/module-map.md
        docs/how-to.md
        docs/mcp.md
    )
fi

missing=0
for DOC in "${DOCS[@]}"; do
    if [[ ! -f "$DOC" ]]; then
        echo "check-doc-paths: $DOC not found" >&2
        exit 2
    fi

    while IFS= read -r path; do
        if [[ ! -e "$path" ]]; then
            echo "check-doc-paths: missing path referenced in $DOC: $path" >&2
            missing=$((missing + 1))
        fi
    done < <(grep -oE 'src/[A-Za-z0-9_/.-]+\.rs' "$DOC" | sort -u)
done

if (( missing > 0 )); then
    echo "check-doc-paths: $missing missing path(s)" >&2
    exit 1
fi

echo "check-doc-paths: all paths exist"
