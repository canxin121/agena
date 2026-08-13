#!/usr/bin/env python3
"""Show pricing + knowledge_cutoff formats from existing entries, and dump the
exact domestic pricing-gap list excluding non-Chinese families (grok/granite)."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

# existing pricing examples
print("== pricing examples ==")
for m in ["deepseek-v4", "qwen3-max", "glm-4.7", "kimi-k3", "doubao-seed-1-6-250615",
          "hunyuan-turbo", "minimax-m2", "step-2", "ernie-5.1", "yi-lightning"]:
    e = cat.get(m)
    if e:
        print(f"  {m}: {json.dumps(e.get('pricing'), ensure_ascii=False)}")

print("\n== knowledge_cutoff examples ==")
for m in ["deepseek-v4", "qwen3-max", "glm-4.7", "kimi-k3", "doubao-seed-1-6-250615",
          "hunyuan-turbo", "minimax-m2", "step-2", "ernie-5.1", "yi-lightning"]:
    e = cat.get(m)
    if e:
        print(f"  {m}: {e.get('knowledge_cutoff')!r}")
