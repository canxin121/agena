#!/usr/bin/env python3
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.json")))["models"]
md = json.load(open(os.path.join(D, "models-dev.json")))
dev = {}
for prov in md.values():
    for mid, m in (prov.get("models") or {}).items():
        real = m.get("id") or mid
        dev.setdefault(real, m)

# Find models where catalog lacks knowledge but models.dev has it
candidates = []
for cid, cdef in cat.items():
    if cdef.get("knowledge_cutoff") is None:
        m = dev.get(cid) or dev.get(cid.lower())
        if m and m.get("knowledge"):
            candidates.append((cid, m.get("knowledge")))
print(f"catalog models missing knowledge_cutoff but models.dev HAS knowledge: {len(candidates)}")
for cid, k in candidates[:15]:
    print("  ", cid, "->", k)

# Also for description
desc = []
for cid, cdef in cat.items():
    if cdef.get("description") is None:
        m = dev.get(cid) or dev.get(cid.lower())
        if m and m.get("description"):
            desc.append((cid, m.get("description")))
print(f"missing description but models.dev has it: {len(desc)}")

# pricing
pr = []
for cid, cdef in cat.items():
    if cdef.get("pricing") is None:
        m = dev.get(cid) or dev.get(cid.lower())
        if m and m.get("cost"):
            pr.append(cid)
print(f"missing pricing but models.dev has cost: {len(pr)}")
print(pr[:20])
