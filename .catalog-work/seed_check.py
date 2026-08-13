#!/usr/bin/env python3
"""Show all catalog models containing 'seed-2' or '2-0' to assess doubao-seed-2-0 coverage."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
for m in sorted(cat):
    if "seed-2" in m or "seed2" in m or "2-0" in m:
        e = cat[m]
        print(f"{m:48s} ctx={e.get('context_window_tokens')} desc={(e.get('description') or '')[:45]}")
