#!/usr/bin/env python3
"""Print ALL 191 missing-description models grouped, to catch any real domestic LLM."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
missing = [m for m, e in cat.items() if not e.get("description")]
for m in sorted(missing):
    e = cat[m]
    print(f"{m:55s} open={e.get('open_weights')} ctx={e.get('context_window_tokens')}")
