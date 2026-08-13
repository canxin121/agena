#!/usr/bin/env python3
"""Cross-reference: domestic models present in models.dev but absent from the
catalog (by exact id). Highlight recent/major ones that look like real gaps."""
import json, os, re

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
md = json.load(open(os.path.join(D, "research", "models.dev.live.json")))
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

# catalog ids (lowercased) + family stems
cat_ids = set(m.lower() for m in cat)

DOMESTIC_FAM = ["deepseek","qwen","qwq","glm","zhipu","mimo","kimi","moonshot",
    "doubao","byteplus","seed-","ernie","wenxin","baichuan","yi-","step",
    "hunyuan","minimax","mini-max","spark","internlm","xihe","tele-","ling",
    "pangu","autoglm","cogito","dola","muse-","umans","zai-","z-ai","coding-",
    "alicloud","alibaba-","grayline","tim-","qwenlong","baichuan4","qwen3guard",
    "xiaomi"]

seen = set()
missing = []
for prov, pv in md.items():
    if not isinstance(pv, dict) or "models" not in pv:
        continue
    for mid, m in pv["models"].items():
        low = mid.lower()
        if low in seen:
            continue
        seen.add(low)
        if not any(f in low for f in DOMESTIC_FAM):
            continue
        # exact absence from catalog
        if low not in cat_ids:
            missing.append((mid, prov, m.get("limit", {}).get("context"),
                            m.get("release_date"), m.get("reasoning")))

# dedupe by mid (keep first)
uniq = {}
for mid, prov, ctx, rd, rsn in sorted(missing):
    if mid not in uniq:
        uniq[mid] = (prov, ctx, rd, rsn)
print(f"domestic models in models.dev but NOT in catalog (by exact id): {len(uniq)}")
print("\n-- by family --")
fam = {}
for mid, (prov, ctx, rd, rsn) in uniq.items():
    f = mid.split("/")[-1].split("-")[0].split(".")[0]
    fam.setdefault(f, []).append((mid, prov, ctx, rd))
for f, ms in sorted(fam.items()):
    print(f"\n== {f} ({len(ms)}) ==")
    for mid, prov, ctx, rd in ms[:25]:
        print(f"   {mid:55s} ctx={ctx} rd={rd} prov={prov}")
