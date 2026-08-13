#!/usr/bin/env python3
"""Survey ALL missing-description models, flag which are domestic vs special."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

DOMESTIC = ["deepseek","qwen","qwq","glm","zhipu","mimo","kimi","moonshot",
    "doubao","byteplus","seed-","ernie","wenxin","baichuan","yi-","step",
    "hunyuan","minimax","mini-max","spark","internlm","xihe","tele-","ling",
    "pangu","autoglm","cogito","dola","muse-","umans","zai-","z-ai","coding-",
    "alicloud","alibaba-","grayline","tim-","qwenlong","baichuan4","qwen3guard"]

missing = [m for m, e in cat.items() if not e.get("description")]
print(f"total missing description: {len(missing)}")
dom = [m for m in missing if any(d in m.lower() for d in DOMESTIC)]
special = [m for m in missing if any(s in m for s in ("granite","-embed","reranker","vision","-tts","-asr","geospatial","timeseries","docling","nvclip","kosmos","vila","deplot","imagen","clip","tokenizer"))]
other = [m for m in missing if m not in dom and m not in special]
print(f"domestic: {len(dom)} {dom}")
print(f"special:  {len(special)}")
print(f"other:    {len(other)} {other[:40]}")
