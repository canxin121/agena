#!/usr/bin/env python3
"""Show existing description style for reference families."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
for m in ["codellama-70b-instruct-hf", "starcoder2-15b-instruct-v0.1", "deepseek-coder-6.7b-instruct",
          "mixtral-8x7b-v0.1", "llama-3.1-8b-instruct", "gemma-3-4b-it", "phi-3-vision-128k-instruct",
          "jamba-reasoning-3b", "mistral-7b", "qwen-2.5-7b-instruct", "fuyu-8b"]:
    e = cat.get(m)
    if e and e.get("description"):
        print(f"{m:42s} {e['description'][:110]}")
