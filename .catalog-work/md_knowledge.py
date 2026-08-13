#!/usr/bin/env python3
"""Check models.dev `knowledge` field: format + coverage of the 225 domestic
knowledge_cutoff gaps."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
md = json.load(open(os.path.join(D, "research", "models.dev.live.json")))
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

# index models.dev by id
index = {}
for prov, pv in md.items():
    if not isinstance(pv, dict) or "models" not in pv:
        continue
    for mid, m in pv["models"].items():
        if mid not in index:
            index[mid] = m

# sample knowledge formats
print("== knowledge samples ==")
for mid in list(index)[:2000]:
    k = index[mid].get("knowledge")
    if k:
        print(f"  {mid:40s} {json.dumps(k, ensure_ascii=False)[:120]}")
        break

# domestic cutoff-gap list (from survey)
DOMESTIC = [
    "deepseek", "qwen", "qwq", "glm", "zhipu", "mimo", "kimi", "moonshot",
    "doubao", "byteplus", "seed", "ernie", "wenxin", "baichuan", "yi-", "step",
    "hunyuan", "minimax", "mini-max", "spark", "internlm", "xihe", "tele",
    "sensenova", "cogvideo", "funaudio", "ling", "pangu", "autoglm", "cogito",
    "dola", "muse", "umans", "zai", "z-ai", "coding-", "alicloud", "alibaba",
    "grayline", "tim-", "qwenlong", "baichuan4", "bge", "qwen3guard", "bge",
]
def is_domestic(mid):
    low = mid.lower()
    return any(p in low for p in DOMESTIC)

gaps = [m for m, e in cat.items()
        if not e.get("knowledge_cutoff") and is_domestic(m)
        and not m.lower().startswith(("grok", "granite"))]
print(f"\ndomestic cutoff gaps (excl grok/granite): {len(gaps)}")

in_md = [m for m in gaps if m in index]
print(f"of those, in models.dev: {len(in_md)}")
for m in sorted(in_md)[:60]:
    k = index[m].get("knowledge")
    print(f"  {m:48s} md_knowledge={json.dumps(k, ensure_ascii=False)[:80]}")
