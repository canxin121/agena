#!/usr/bin/env python3
"""Dump remaining missing-cap models with a hint of what they are."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
res = json.load(open(os.path.join(D, "research", "hf_configs.json")))
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
fetched = res["fetched"]
failed = res["failed"]
print("fetched", len(fetched), "failed", len(failed))
for m in sorted(failed):
    e = cat[m]
    print(m, "|", (e.get("description") or "")[:70])
