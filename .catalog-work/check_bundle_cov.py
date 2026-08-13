#!/usr/bin/env python3
"""After the capability fix, recheck which reasoning-gap models exist now and
compare against what the bundles contained. Report any newly-appeared gap
models not covered by any bundle (so I can patch them separately)."""
import json, os, glob
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(D + "/models.merged.json"))["models"]

def reasoning_supported(c):
    feats = c.get("features")
    if isinstance(feats, list):
        return "reasoning" in feats
    if isinstance(feats, dict):
        return "reasoning" in (feats.get("supported") or [])
    return False

# all models currently reasoning-supported without thinking modes
cur_gap = set()
for mid, c in cat.items():
    if reasoning_supported(c) and not (c.get("thinking_modes") or {}):
        cur_gap.add(mid)

# what bundles covered
bundled = set()
for pf in glob.glob(D + "/bundle_entries/*.json"):
    b = json.load(open(pf))
    bundled.update(b.keys())

missing_from_bundles = sorted(cur_gap - bundled)
extra_in_bundles = sorted(bundled - cur_gap)
print(f"current reasoning-gap: {len(cur_gap)}")
print(f"bundled: {len(bundled)}")
print(f"gap models NOT in any bundle (need handling): {len(missing_from_bundles)}")
for m in missing_from_bundles:
    c = cat[m]
    print("  ", m, "| origin:", c.get("origin"))
print(f"\nbundle models no longer reasoning-gap: {len(extra_in_bundles)}")
for m in extra_in_bundles[:20]:
    print("  ", m)
