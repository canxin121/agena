#!/usr/bin/env python3
"""Definitive reasoning-gap report for subagent bundling, from merged catalog."""
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

def reasoning_supported(c):
    feats = c.get("capabilities", {}).get("features") if c.get("capabilities") else c.get("features")
    if isinstance(feats, list):
        return "reasoning" in feats
    if isinstance(feats, dict):
        return "reasoning" in (feats.get("supported") or [])
    return False

gap = {}
for mid, c in cat.items():
    if reasoning_supported(c) and not (c.get("thinking_modes") or {}):
        gap.setdefault(c.get("origin"), []).append(mid)

# write per-origin files
os.makedirs(os.path.join(D, "reason_gap"), exist_ok=True)
for org, ids in gap.items():
    with open(os.path.join(D, "reason_gap", org.replace("/", "_") + ".txt"), "w") as f:
        f.write("\n".join(sorted(ids)) + "\n")

tot = sum(len(v) for v in gap.values())
print(f"total reasoning-gap models: {tot}, origins: {len(gap)}")
for org, ids in sorted(gap.items(), key=lambda kv: -len(kv[1])):
    print(f"{org}\t{len(ids)}\t{','.join(ids[:5])}")
