#!/usr/bin/env python3
"""Post-merge coverage report vs baseline."""
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

def reasoning_supported(c):
    cap = c.get("capabilities") or {}
    feats = cap.get("features") if isinstance(cap, dict) else None
    if feats is None:
        feats = c.get("features")
    if isinstance(feats, list):
        return "reasoning" in feats
    if isinstance(feats, dict):
        return "reasoning" in (feats.get("supported") or [])
    return False

n = len(cat)
def miss(f):
    return sum(1 for c in cat.values() if c.get(f) is None)

print(f"total models: {n}")
for f in ("description", "context_window_tokens", "max_input_tokens", "max_output_tokens", "pricing", "knowledge_cutoff"):
    print(f"missing {f}: {miss(f)}")
print(f"thinking_modes present: {sum(1 for c in cat.values() if c.get('thinking_modes'))}")
print(f"speed_modes present: {sum(1 for c in cat.values() if c.get('speed_modes'))}")
print(f"capabilities present: {n - miss('capabilities')}")
rs = sum(1 for c in cat.values() if reasoning_supported(c))
rs_no_tm = sum(1 for c in cat.values() if reasoning_supported(c) and not c.get('thinking_modes'))
print(f"reasoning-supported: {rs}, of which still NO thinking modes: {rs_no_tm}")
