#!/usr/bin/env bash
set -e
D="/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
# For each origin, list models with each missing field, so subagents can focus.
mkdir -p "$D/gaps"
jq -r '.models | to_entries[] | [.key, (.value.origin // "unknown")] | @tsv' "$D/models.json" > "$D/model-origin.tsv"

# Group model ids by origin (canonical list)
while IFS=$'\t' read -r mid origin; do
  echo "$mid" >> "$D/gaps/${origin//[^A-Za-z0-9._-]/_}.txt"
done < "$D/model-origin.tsv"

echo "=== 每个 origin 的模型数 ==="
for f in "$D/gaps"/*.txt; do
  echo "$(basename "$f" .txt): $(wc -l < "$f")"
done | sort -t: -k2 -rn | head -40
