#!/usr/bin/env python3
"""Print full list of missing-description models for the backfill authoring."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
missing = [m for m, e in cat.items() if not e.get("description")]
for m in sorted(missing):
    print(m)
