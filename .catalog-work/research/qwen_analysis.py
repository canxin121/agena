#!/usr/bin/env python3
"""Helper to analyze qwen state in merged catalog + models.dev snapshot."""
import json, os, sys

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
merged = json.load(open(os.path.join(D, "models.merged.json")))["models"]
dev = json.load(open(os.path.join(D, "research/models.dev.json")))
devmap = {}
for prov in dev.values():
    for mid, m in (prov.get("models") or {}).items():
        real = m.get("id") or mid
        devmap.setdefault(real, m)
        devmap.setdefault(mid, m)

qwen = {k: v for k, v in merged.items() if k.startswith("qwen") or k.startswith("alibaba")}

mode = sys.argv[1] if len(sys.argv) > 1 else "thinking-variants"

if mode == "thinking-variants":
    for k, v in sorted(qwen.items()):
        if ("thinking" in k.lower() or ":thinking" in k) and not v.get("thinking_modes"):
            print("THINKING VARIANT WITHOUT MODES:", k, "| desc:", (v.get("description") or "")[:70])

elif mode == "kcutoff-dev":
    # models.dev knowledge_cutoff for all qwen models missing it in catalog
    for k in sorted(qwen.keys()):
        if qwen[k].get("knowledge_cutoff"):
            continue
        dm = devmap.get(k)
        kc = (dm or {}).get("knowledge_cutoff")
        rel = (dm or {}).get("release_date")
        if kc:
            print(f"{k}: dev kcutoff={kc} release={rel}")
    print("--- models with NO dev kcutoff among missing ---")
    for k in sorted(qwen.keys()):
        if qwen[k].get("knowledge_cutoff"):
            continue
        dm = devmap.get(k)
        kc = (dm or {}).get("knowledge_cutoff")
        if not kc:
            print(k)

elif mode == "open-weights":
    for k, v in sorted(qwen.items()):
        print(f"{k}: open_weights={v.get('open_weights')} | desc={(v.get('description') or '')[:60]}")

elif mode == "ropts-missing-think":
    # models flagged think? whose models.dev has reasoning_options
    for k, v in sorted(qwen.items()):
        if v.get("thinking_modes"):
            continue
        dm = devmap.get(k)
        ropts = (dm or {}).get("reasoning_options")
        if ropts:
            print(f"{k}: dev ropts={json.dumps(ropts, ensure_ascii=False)}")

elif mode == "dev-full":
    # dump full models.dev entries for a set of IDs
    for k in sys.argv[2:]:
        dm = devmap.get(k)
        if dm:
            keep = {kk: dm.get(kk) for kk in
                    ["context_length","max_output_tokens","reasoning_options","knowledge_cutoff",
                     "release_date","parameters","input_modalities","output_modalities","pricing"]}
            print(f"=== {k} ===")
            print(json.dumps(keep, ensure_ascii=False, indent=1))
        else:
            print(f"=== {k}: NO DEV ENTRY ===")
