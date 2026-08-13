#!/usr/bin/env python3
"""Fuzzy-check the 7 NOT-found models for alias variants in the catalog."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
ids = list(cat.keys())

TARGETS = ["deepseek-v4-latest", "glm-5v", "kimi-k2.7", "doubao-seed-2-0-lite",
           "doubao-seed-2-0-code", "qwen3.5-max", "hunyuan-2.0"]
for t in TARGETS:
    print(f"\n== {t} ==")
    parts = t.replace(".", "-").split("-")
    for i, p in enumerate(parts):
        hits = [m for m in ids if p in m]
        if hits:
            print(f"  '{p}' -> {hits[:12]}")
            break
