#!/usr/bin/env python3
"""Survey domestic-model gaps (description / pricing / knowledge_cutoff) across
the catalog. Domestic families defined by ID prefix."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

# domestic family prefixes (from the 10-agent domestic sweep)
DOMESTIC = [
    "deepseek", "qwen", "qwq", "glm", "zhipu", "mimo", "kimi", "moonshot",
    "doubao", "byteplus", "seed", "ernie", "wenxin", "baichuan", "yi-", "step",
    "hunyuan", "minimax", "mini-max", "spark", "internlm", "xihe", "tele",
    "sensenova", "cogvideo", "funaudio", "ling", "pangu", "gtp", "grok",
]

def is_domestic(mid):
    low = mid.lower()
    for p in DOMESTIC:
        if low.startswith(p) or p in low:
            return True
    return False

dom = {m: e for m, e in cat.items() if is_domestic(m)}
print(f"domestic models: {len(dom)} / {len(cat)}")

gaps = {"description": [], "pricing": [], "knowledge_cutoff": [], "caps": []}
for m, e in sorted(dom.items()):
    if not e.get("description"):
        gaps["description"].append(m)
    if not e.get("pricing"):
        gaps["pricing"].append(m)
    if not e.get("knowledge_cutoff"):
        gaps["knowledge_cutoff"].append(m)
    if e.get("context_window_tokens") is None:
        gaps["caps"].append(m)

for k, v in gaps.items():
    print(f"\n== {k}: {len(v)} ==")
    # group by family prefix
    fam = {}
    for m in v:
        f = m.split("-")[0].split(".")[0].split("_")[0]
        fam.setdefault(f, []).append(m)
    for f, ms in sorted(fam.items()):
        print(f"  {f:14s} {len(ms):3d}  {', '.join(ms[:8])}{' ...' if len(ms)>8 else ''}")
