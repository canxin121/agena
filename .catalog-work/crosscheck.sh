#!/usr/bin/env bash
set -e
D="/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
# Build a map: models.dev provider -> {model_id -> {fields}}
# models.dev shape: { provider_id: { id, name, models: { model_id: {...} } } }
# Compare against catalog model ids (canonical).

jq -r '
  to_entries[] as $p
  | $p.value.models // {} | to_entries[] as $m
  | [$p.key, $m.key, ($m.value | tostring)]
  | @tsv
' "$D/models-dev.json" > "$D/models-dev-flat.tsv"
wc -l "$D/models-dev-flat.tsv"

# Now for each catalog model id, look up models.dev entries (by exact key OR normalized).
echo "=== catalog 模型在 models.dev 的精确命中 ==="
jq -r '.models | keys[]' "$D/models.json" | while read -r id; do
  if grep -qP "\t${id}\t" "$D/models-dev-flat.tsv"; then echo "$id"; fi
done > "$D/matched-ids.txt"
wc -l < "$D/matched-ids.txt"
