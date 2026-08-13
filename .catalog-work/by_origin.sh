#!/usr/bin/env bash
set -e
F="/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/models.json"
echo "=== 按 origin 统计: 模型数 / 缺pricing / 缺description / 缺context / 缺thinking / 缺speed ==="
jq -r '.models | to_entries | group_by(.value.origin) | map({origin: .[0].value.origin, n: length,
  no_pricing: ([.[] | select(.value.pricing == null)] | length),
  no_desc: ([.[] | select(.value.description == null)] | length),
  no_ctx: ([.[] | select(.value.context_window_tokens == null)] | length),
  no_think: ([.[] | select(((.value.thinking_modes // {}) | length) == 0)] | length),
  no_speed: ([.[] | select(((.value.speed_modes // {}) | length) == 0)] | length)}) |
  sort_by(-.n) | .[] | "\(.origin): n=\(.n) 缺pricing=\(.no_pricing) 缺desc=\(.no_desc) 缺ctx=\(.no_ctx) 缺think=\(.no_think) 缺speed=\(.no_speed)"' "$F"
