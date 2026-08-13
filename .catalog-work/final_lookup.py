#!/usr/bin/env python3
"""Look up specific domestic models in models.dev for the last fillable fields:
glm-5v existence, glm-4.7-n cost, kimi-k3.256k, deepseek-coder-6.7b."""
import json, os

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

for want in ["glm-5v", "glm-4.7-n", "kimi-k3.256k", "deepseek-coder-6.7b-instruct",
             "glm-4.7", "kimi-k3.256k", "doubao-seed-2-0-pro-260215", "seed-2-0-code"]:
    print(f"\n== {want} ==")
    # exact
    if want in index:
        for prov, m in index[want][:3]:
            print(f"  EXACT {prov}: {json.dumps(m, ensure_ascii=False)[:400]}")
    # substring
    hits = [(mid, prov, m) for mid, recs in index.items() for prov, m in recs if want.lower() in mid.lower()]
    if hits and want not in index:
        print(f"  {len(hits)} substring hits:")
        for mid, prov, m in hits[:8]:
            print(f"    {mid:50s} prov={prov:18s} cost={m.get('cost')} lim={m.get('limit')} rd={m.get('release_date')}")
