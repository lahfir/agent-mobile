#!/usr/bin/env bash
# Rust source rules that clippy cannot express: 400 lines per file, no inline comments,
# doc comments of at most 15 lines. Run from the repo root; CI and the pre-commit hook call it.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

limit=400
failed=0

rs_files=()
while IFS= read -r -d '' file; do
    rs_files+=("$file")
done < <(git ls-files -z --cached --others --exclude-standard -- '*.rs')

for file in "${rs_files[@]}"; do
    [ -f "$file" ] || continue
    if head -n 5 "$file" | grep -q '@generated'; then
        continue
    fi
    lines="$(wc -l < "$file" | tr -d ' ')"
    if [ "$lines" -gt "$limit" ]; then
        printf '%s: %s lines (limit %s)\n' "$file" "$lines" "$limit" >&2
        failed=1
    fi
done

if ! printf '%s\0' "${rs_files[@]}" | python3 scripts/check_rust_comments.py; then
    failed=1
fi

exit "$failed"
