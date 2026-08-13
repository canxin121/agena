#!/usr/bin/env bash
set -e
F="/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/models.json"
# reasoning supported if: features is array containing "reasoning", or object whose supported contains "reasoning"
echo "=== 推理模型(reasoning supported)但无 thinking_modes ==="
jq -r '.models | to_entries[] | select(
  (((.value.features | type) == "array" and (.value.features | index("reasoning"))) or
   ((.value.features | type) == "object" and ((.value.features.supported // []) | index("reasoning"))))
  and ((.value.thinking_modes // {}) | length == 0)) | "\(.key) [\(.value.origin)]"' "$F" | head -80
echo "数量:"
jq '[.models | to_entries[] | select(
  (((.value.features | type) == "array" and (.value.features | index("reasoning"))) or
   ((.value.features | type) == "object" and ((.value.features.supported // []) | index("reasoning"))))
  and ((.value.thinking_modes // {}) | length == 0))] | length' "$F"
echo ""
echo "=== 按 origin 的推理缺口 ==="
jq -r '.models | to_entries[] | select(
  (((.value.features | type) == "array" and (.value.features | index("reasoning"))) or
   ((.value.features | type) == "object" and ((.value.features.supported // []) | index("reasoning"))))
  and ((.value.thinking_modes // {}) | length == 0)) | .value.origin' "$F" | sort | uniq -c | sort -rn
