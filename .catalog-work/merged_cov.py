#!/usr/bin/env python3
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

def reasoning_supported(c):
    feats = c.get("features")
    if isinstance(feats, list):
        return "reasoning" in feats
    if isinstance(feats, dict):
        return "reasoning" in (feats.get("supported") or [])
    return False

rs = sum(1 for c in cat.values() if reasoning_supported(c))
print(f"reasoning-supported: {rs}")
print(f"has thinking modes: {sum(1 for c in cat.values() if c.get('thinking_modes'))}")
print(f"has speed modes: {sum(1 for c in cat.values() if c.get('speed_modes'))}")
# capabilities still null
print(f"capabilities fully null: {sum(1 for c in cat.values() if c.get('capabilities') is None)}")
print(f"input+features null: {sum(1 for c in cat.values() if c.get('input') is None and c.get('features') is None)}")
