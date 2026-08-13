#!/usr/bin/env python3
"""Survey OTHER domestic gaps: release_date / open_weights / last_updated /
speed_modes / supports_parallel_tool_calls, plus known-missing major models."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

DOMESTIC = ["deepseek","qwen","qwq","glm","zhipu","mimo","kimi","moonshot",
    "doubao","byteplus","seed-","ernie","wenxin","baichuan","yi-","step",
    "hunyuan","minimax","mini-max","spark","internlm","xihe","tele-","ling",
    "pangu","autoglm","cogito","dola","muse-","umans","zai-","z-ai","coding-",
    "alicloud","alibaba-","grayline","tim-","qwenlong","baichuan4","qwen3guard"]
def is_domestic(mid):
    low = mid.lower()
    return any(p in low for p in DOMESTIC)

dom = {m: e for m, e in cat.items() if is_domestic(m) and not m.lower().startswith(("grok","granite"))}
print(f"domestic (excl grok/granite): {len(dom)}")

for field in ("release_date", "open_weights", "last_updated", "speed_modes",
              "supports_parallel_tool_calls", "default_temperature"):
    gaps = [m for m, e in dom.items() if e.get(field) is None]
    print(f"\n== missing {field}: {len(gaps)} ==")
    fam = {}
    for m in gaps:
        f = m.split("-")[0].split(".")[0]
        fam.setdefault(f, []).append(m)
    for f, ms in sorted(fam.items()):
        print(f"  {f:14s} {len(ms):3d}  {', '.join(ms[:6])}{' ...' if len(ms)>6 else ''}")
