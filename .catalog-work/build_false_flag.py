#!/usr/bin/env python3
"""Fix the 11 models where the merged catalog's `features` wrongly claims
reasoning support (models.dev authoritative `reasoning=false`):
  - 10 non-reasoning: remove 'reasoning' from features.supported
  - grok-4: keep reasoning; add family-consistent effort ladder (mirrors
    grok-4.0709 / grok-4.5 which are verified xAI thinking models)
Writes patches/false_flag_fix.json."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

# models to remove 'reasoning' from supported (models.dev reasoning=False,
# no thinking control; base instruct / VL / coder / fast-lane models)
REMOVE = [
    "apertus-70b",
    "gemma-3-27b-it",
    "grok-code-fast-1",
    "llama-3.1-8b-instruct",
    "llama-3.2-3b-instruct",
    "pixtral-12b-2409",
    "qvq-max",
    "qwen-turbo",
    "qwen2.5-vl-72b-instruct",
    "qwen3-coder-next",
]

patch_models = {}
for mid in REMOVE:
    x = cat.get(mid)
    if x is None:
        print(f"WARN {mid}: absent, skip"); continue
    feats = x.get("features")
    if not isinstance(feats, dict):
        print(f"WARN {mid}: features not dict ({feats!r}), skip"); continue
    sup = list(feats.get("supported") or [])
    if "reasoning" not in sup:
        print(f"NOTE {mid}: reasoning already absent"); continue
    new_sup = [f for f in sup if f != "reasoning"]
    patch_models[mid] = {"features": {"supported": new_sup,
                                       "unsupported": feats.get("unsupported") or []}}

# grok-4: effort ladder mirroring verified xAI family (grok-4.0709 / grok-4.5)
grok4 = cat.get("grok-4")
if grok4 is None:
    print("WARN grok-4 absent")
else:
    patch_models["grok-4"] = {"thinking_modes": {
        "high": {"display_name": "Think High", "strategy": "effort", "effort": "high"},
        "low": {"display_name": "Think Low", "strategy": "effort", "effort": "low"},
        "medium": {"display_name": "Think Medium", "strategy": "effort", "effort": "medium"},
    }}

patch = {
    "models": patch_models,
    "notes": (
        "Remove spurious 'reasoning' feature from 10 non-reasoning models (models.dev "
        "authoritative reasoning=false: apertus-70b, gemma-3-27b-it, grok-code-fast-1 "
        "fast lane, llama-3.1-8b / 3.2-3b instruct, pixtral-12b-2409, qvq-max, qwen-turbo, "
        "qwen2.5-vl-72b, qwen3-coder-next). grok-4 keeps reasoning and gets the xAI "
        "family effort ladder (grok-4.0709 / grok-4.5 verified)."
    ),
}
out = os.path.join(D, "patches", "false_flag_fix.json")
json.dump(patch, open(out, "w"), ensure_ascii=False, indent=2)
print(f"wrote {len(patch_models)} models -> {os.path.basename(out)}")
