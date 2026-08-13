#!/usr/bin/env bash
set -e
F="/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/models.json"
echo "总模型: $(jq '.models | length' "$F")"
echo "=== 各字段缺失数量 ==="
for f in description context_window_tokens max_input_tokens max_output_tokens pricing display_name knowledge_cutoff release_date open_weights origin; do
  c=$(jq --arg f "$f" '[.models[] | select(.[$f] == null)] | length' "$F")
  echo "$f: 缺失 $c"
done
echo "capabilities 全缺(input+features): $(jq '[.models[] | select((.input == null) and (.features == null))] | length' "$F")"
echo "thinking_modes 非空: $(jq '[.models[] | select((.thinking_modes // {}) | length > 0)] | length' "$F")"
echo "speed_modes 非空: $(jq '[.models[] | select((.speed_modes // {}) | length > 0)] | length' "$F")"
echo "thinking+speed 都空: $(jq '[.models[] | select(((.thinking_modes // {}) | length == 0) and ((.speed_modes // {}) | length == 0))] | length' "$F")"
echo "=== 主要厂商缺失情况 ==="
echo "OpenAI 缺失 description: $(jq '[.models[] | select(.origin == "OpenAI" and .description == null)] | length' "$F")"
echo "OpenAI 缺失 pricing: $(jq '[.models[] | select(.origin == "OpenAI" and .pricing == null)] | length' "$F")"
echo "Anthropic 缺失 description: $(jq '[.models[] | select(.origin == "Anthropic" and .description == null)] | length' "$F")"
echo "Google 缺失 description: $(jq '[.models[] | select(.origin == "Google" and .description == null)] | length' "$F")"
echo "DeepSeek 缺失 description: $(jq '[.models[] | select(.origin == "DeepSeek" and .description == null)] | length' "$F")"
