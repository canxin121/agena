#!/usr/bin/env bash
set -e
F="/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/models.merged.json"
echo "总模型: $(jq '.models | length' "$F")"
echo "=== 各字段缺失数量 (合并后) ==="
for f in description context_window_tokens max_input_tokens max_output_tokens pricing knowledge_cutoff; do
  c=$(jq --arg f "$f" '[.models[] | select(.[$f] == null)] | length' "$F")
  echo "$f: 缺失 $c"
done
echo "capabilities 全缺: $(jq '[.models[] | select((.input == null) and (.features == null))] | length' "$F")"
echo "thinking_modes 非空: $(jq '[.models[] | select((.thinking_modes // {}) | length > 0)] | length' "$F")"
echo "speed_modes 非空: $(jq '[.models[] | select((.speed_modes // {}) | length > 0)] | length' "$F")"
echo "thinking+speed 都空: $(jq '[.models[] | select(((.thinking_modes // {}) | length == 0) and ((.speed_modes // {}) | length == 0))] | length' "$F")"
