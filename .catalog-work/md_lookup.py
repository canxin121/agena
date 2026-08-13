#!/usr/bin/env python3
"""Look up models.dev limit.context / limit.output for catalog models,
indexing across all providers."""
import json, os, sys

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
md = json.load(open(os.path.join(D, "research", "models.dev.live.json")))

index = {}
for prov, pv in md.items():
    if not isinstance(pv, dict) or "models" not in pv:
        continue
    for mid, m in pv["models"].items():
        if mid not in index:
            index[mid] = []
        index[mid].append((prov, m))

print("indexed distinct model ids:", len(index))

def lookup(substr, limit=16):
    hits = [(mid, recs) for mid, recs in index.items() if substr.lower() in mid.lower()]
    print(f"\n== {substr} ({len(hits)}) ==")
    for mid, recs in sorted(hits)[:limit]:
        prov, m = recs[0]
        lim = m.get("limit") or {}
        print(f"  {mid:45s} ctx={lim.get('context')} out={lim.get('output')} "
              f"in={m.get('modalities',{}).get('input')} outm={m.get('modalities',{}).get('output')} prov={prov}")

for s in sys.argv[1:]:
    lookup(s)
