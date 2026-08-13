#!/usr/bin/env python3
"""Show which reasoning-capable models lack thinking modes, grouped by origin,
and list all present speed/thinking mode keys for reference."""
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.json")))["models"]

def reasoning_supported(c):
    feats = c.get("features")
    if isinstance(feats, list):
        return "reasoning" in feats
    if isinstance(feats, dict):
        return "reasoning" in (feats.get("supported") or [])
    return False

gap = {}
have = {}
for mid, c in cat.items():
    rs = reasoning_supported(c)
    has_t = bool(c.get("thinking_modes") or {})
    if rs and not has_t:
        gap.setdefault(c.get("origin"), []).append(mid)
    if has_t:
        have.setdefault(c.get("origin"), []).append(mid)

print("=== reasoning-supported but NO thinking modes (by origin) ===")
tot = 0
for org, ids in sorted(gap.items(), key=lambda kv: -len(kv[1])):
    tot += len(ids)
    print(f"{org}: {len(ids)}  e.g. {ids[:6]}")
print("total:", tot)

print("\n=== origins that HAVE thinking modes (reference) ===")
for org, ids in sorted(have.items(), key=lambda kv: -len(kv[1])):
    print(f"{org}: {len(ids)}  e.g. {ids[:4]}")
