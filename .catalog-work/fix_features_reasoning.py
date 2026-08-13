#!/usr/bin/env python3
"""Remove spurious 'reasoning' from features.supported for 10 non-reasoning
models (models.dev authoritative reasoning=false). Directly edits models.merged.json
since `features` is a merged capability layer, not a patch-allowed field."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
CAT = os.path.join(D, "models.merged.json")

REMOVE = [
    "apertus-70b", "gemma-3-27b-it", "grok-code-fast-1",
    "llama-3.1-8b-instruct", "llama-3.2-3b-instruct", "pixtral-12b-2409",
    "qvq-max", "qwen-turbo", "qwen2.5-vl-72b-instruct", "qwen3-coder-next",
]

doc = json.load(open(CAT))
models = doc["models"]
changed = 0
for mid in REMOVE:
    x = models.get(mid)
    if x is None:
        print(f"WARN {mid}: absent, skip"); continue
    feats = x.get("features")
    if not isinstance(feats, dict):
        print(f"WARN {mid}: features not dict ({feats!r}), skip"); continue
    sup = feats.get("supported") or []
    if "reasoning" not in sup:
        print(f"NOTE {mid}: reasoning already absent"); continue
    feats["supported"] = [f for f in sup if f != "reasoning"]
    changed += 1

json.dump(doc, open(CAT, "w"), ensure_ascii=False, indent=2)
print(f"removed reasoning flag from {changed} models")
