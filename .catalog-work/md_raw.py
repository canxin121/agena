#!/usr/bin/env python3
"""Dump one full models.dev model entry + collect the union of keys used."""
import json, os, collections

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
md = json.load(open(os.path.join(D, "research", "models.dev.live.json")))

keys = collections.Counter()
shown = 0
for prov, pv in md.items():
    if not isinstance(pv, dict) or "models" not in pv:
        continue
    for mid, m in pv["models"].items():
        for k in m:
            keys[k] += 1
        if shown < 1:
            print("SAMPLE", prov, mid)
            print(json.dumps(m, indent=2, ensure_ascii=False)[:1800])
            shown += 1

print("\nALL KEYS USED:")
for k, c in keys.most_common():
    print(f"  {k:35s} {c}")
