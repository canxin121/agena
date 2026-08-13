#!/usr/bin/env python3
"""Show full entries (with id keys) for OpenAI/xAI speed modes."""
import json
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(D + "/models.merged.json"))["models"]

shown = 0
for mid, m in cat.items():
    if m.get("speed_modes") and m.get("origin") in ("OpenAI", "xAI") and shown < 4:
        print("="*70)
        print("KEY:", mid)
        print(json.dumps(m.get("speed_modes"), indent=1)[:1500])
        shown += 1
