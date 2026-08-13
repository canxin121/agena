#!/usr/bin/env python3
"""Which domestic cutoff-gap models have NON-NULL models.dev knowledge?"""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
md = json.load(open(os.path.join(D, "research", "models.dev.live.json")))
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

index = {}
for prov, pv in md.items():
    if not isinstance(pv, dict) or "models" not in pv:
        continue
    for mid, m in pv["models"].items():
        if mid not in index:
            index[mid] = m

DOMESTIC = ["deepseek","qwen","qwq","glm","zhipu","mimo","kimi","moonshot",
    "doubao","byteplus","seed","ernie","wenxin","baichuan","yi-","step",
    "hunyuan","minimax","mini-max","spark","internlm","xihe","tele",
    "sensenova","cogvideo","funaudio","ling","pangu","autoglm","cogito",
    "dola","muse","umans","zai","z-ai","coding-","alicloud","alibaba",
    "grayline","tim-","qwenlong","baichuan4","bge","qwen3guard"]
def is_domestic(mid):
    low = mid.lower()
    return any(p in low for p in DOMESTIC)

gaps = [m for m, e in cat.items()
        if not e.get("knowledge_cutoff") and is_domestic(m)
        and not m.lower().startswith(("grok", "granite"))]

hits = []
for m in gaps:
    k = index.get(m, {}).get("knowledge")
    if k:
        hits.append((m, k))
print(f"domestic gaps with non-null md knowledge: {len(hits)}")
for m, k in sorted(hits):
    print(f"  {m:48s} {json.dumps(k, ensure_ascii=False)[:60]}")

# also show what the sibling models WITH cutoff look like (the 44 backfilled qwen etc)
print("\n== existing catalog cutoffs in same families (reference) ==")
shown = set()
for m, e in sorted(cat.items()):
    if e.get("knowledge_cutoff") and is_domestic(m) and not m.lower().startswith(("grok","granite")):
        f = m.split("-")[0].split(".")[0]
        if f not in shown and len(shown) < 40:
            shown.add(f)
            print(f"  {f:12s} e.g. {m:44s} -> {e['knowledge_cutoff']}")
