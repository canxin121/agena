#!/usr/bin/env python3
"""Extract current entries for reasoning-gap models per bundle, for subagents.
Also creates the patches/ directory."""
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(D + "/models.merged.json"))["models"]

def reasoning_supported(c):
    cap = c.get("capabilities") or {}
    feats = cap.get("features") if isinstance(cap, dict) else None
    if feats is None:
        feats = c.get("features")
    if isinstance(feats, list):
        return "reasoning" in feats
    if isinstance(feats, dict):
        return "reasoning" in (feats.get("supported") or [])
    return False

gap_models = {}
for mid, c in cat.items():
    if reasoning_supported(c) and not (c.get("thinking_modes") or {}):
        gap_models.setdefault(c.get("origin"), []).append(mid)

# Define bundles: name -> list of origins
bundles = {
    "alibaba": ["Alibaba"],
    "zhipu": ["Zhipu AI", "Zai", "Glm5", "Olafangensan", "Alicloud"],
    "deepseek": ["DeepSeek", "StepFun"],
    "bytedance_moonshot": ["ByteDance", "Moonshot AI", "Umans"],
    "nvidia": ["NVIDIA", "Nvidia", "Nemotron", "Openreasoning", "Sakana", "Liquid AI", "Lfm2"],
    "mistral_minimax": ["Mistral AI", "MiniMax", "Cohere", "Reka"],
    "xiaomi_xai_openai": ["Xiaomi", "xAI", "Xai", "OpenAI"],
    "google_misc_big": ["Google", "Microsoft", "Amazon", "Databricks", "Perplexity", "Upstage", "Baidu", "Tencent", "Sarvam AI"],
}
misc = [o for o in gap_models if o not in set().union(*[set(v) for v in bundles.values()])]
bundles["misc_long_tail"] = misc

os.makedirs(D + "/patches", exist_ok=True)
os.makedirs(D + "/bundle_entries", exist_ok=True)

total = 0
for bname, origins in bundles.items():
    ids = []
    for o in origins:
        ids.extend(gap_models.get(o, []))
    ids = sorted(set(ids))
    total += len(ids)
    with open(D + f"/bundle_entries/{bname}.json", "w") as f:
        json.dump({mid: cat[mid] for mid in ids if mid in cat}, f, ensure_ascii=False, indent=1)
    print(f"{bname}\t{len(ids)}\torigins={origins}")

print("TOTAL", total)
