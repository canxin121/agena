#!/usr/bin/env python3
"""Fold any nested 'capabilities' key into flattened top-level input/features,
removing the nested key. ModelCapabilityPatch is #[serde(flatten)] so the nested
key would be silently dropped on deserialize."""
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
F = os.path.join(D, "models.merged.json")
cat = json.load(open(F))
models = cat["models"]

def union_selection(existing, patch_sel):
    """existing: None | list | {supported, unsupported}
       patch_sel: list | {supported, unsupported}"""
    ex = {}
    if isinstance(existing, dict):
        ex = {"supported": set(existing.get("supported") or []),
              "unsupported": set(existing.get("unsupported") or [])}
    elif isinstance(existing, list):
        ex = {"supported": set(existing), "unsupported": set()}
    else:
        ex = {"supported": set(), "unsupported": set()}
    if isinstance(patch_sel, dict):
        ex["supported"].update(patch_sel.get("supported") or [])
        ex["unsupported"].update(patch_sel.get("unsupported") or [])
    elif isinstance(patch_sel, list):
        ex["supported"].update(patch_sel)
    # A feature can't be both; supported wins
    ex["unsupported"] -= ex["supported"]
    if not ex["supported"] and not ex["unsupported"]:
        return None
    out = {}
    if ex["supported"]:
        out["supported"] = sorted(ex["supported"])
    if ex["unsupported"]:
        out["unsupported"] = sorted(ex["unsupported"])
    return out

fixed = 0
for mid, c in models.items():
    cap = c.get("capabilities")
    if not isinstance(cap, dict):
        continue
    for key in ("input", "features"):
        if key in cap and cap[key] is not None:
            merged = union_selection(c.get(key), cap[key])
            if merged is not None:
                c[key] = merged
    del c["capabilities"]
    fixed += 1

json.dump(cat, open(F, "w"), ensure_ascii=False, indent=2)
print(f"fixed {fixed} models (folded nested capabilities into flat keys)")
