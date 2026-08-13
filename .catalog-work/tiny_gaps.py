#!/usr/bin/env python3
"""Show current state of the last 3 fillable entries + their siblings."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

def show(m):
    e = cat.get(m)
    if not e:
        print(f"  {m}: NOT IN CATALOG")
        return
    print(f"  {m}:")
    for k in ("description","release_date","last_updated","open_weights",
              "context_window_tokens","max_input_tokens","max_output_tokens",
              "pricing","knowledge_cutoff"):
        v = e.get(k)
        if v is not None:
            print(f"    {k}: {json.dumps(v, ensure_ascii=False)[:100]}")

for m in ["kimi-k3.256k", "kimi-k3", "deepseek-coder-6.7b-instruct",
          "deepseek-coder-6.7b", "glm-4.7-n", "glm-4.7"]:
    print()
    show(m)
