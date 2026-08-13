#!/usr/bin/env python3
"""Show speed_modes per origin for reference, plus a couple of full entries with BOTH thinking and speed."""
import json
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(D + "/models.merged.json"))["models"]

by_origin = {}
for m in cat.values():
    if m.get("speed_modes"):
        by_origin.setdefault(m.get("origin"), 0)
        by_origin[m.get("origin")] += 1
print("speed_modes by origin:", by_origin)

# print one full entry that has both thinking and speed, with its id
both = [m for m in cat.values() if m.get("thinking_modes") and m.get("speed_modes")]
print("\nentries with BOTH:", len(both))
for m in both[:3]:
    print("="*70)
    print("id:", m.get("id"), "| origin:", m.get("origin"))
    print(json.dumps({k: m.get(k) for k in ("thinking_modes", "speed_modes")}, indent=1)[:2000])
