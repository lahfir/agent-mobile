#!/usr/bin/env bash
# Rust source rules that clippy cannot express: 400 lines per file, no inline comments,
# doc comments of at most 15 lines. Run from the repo root; CI and the pre-commit hook call it.
set -euo pipefail

limit=400
failed=0

while IFS= read -r file; do
    [ -f "$file" ] || continue
    if head -n 5 "$file" | grep -q '@generated'; then
        continue
    fi
    lines="$(wc -l < "$file" | tr -d ' ')"
    if [ "$lines" -gt "$limit" ]; then
        printf '%s: %s lines (limit %s)\n' "$file" "$lines" "$limit" >&2
        failed=1
    fi
done < <(git ls-files --cached --others --exclude-standard -- '*.rs')

if ! git ls-files -z --cached --others --exclude-standard -- '*.rs' \
    | python3 scripts/check_rust_comments.py; then
    failed=1
fi

exit "$failed"
