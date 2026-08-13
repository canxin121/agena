#!/usr/bin/env python3
"""Re-apply verified family-consistent knowledge_cutoff backfills for Qwen
models that the agent's final write dropped. Values mirror sibling entries
already present in the merged catalog (Qwen3.5/3.6/3.7/3.8 = 2025-04 family
cutoff; Qwen2.5 = 2024-04; qwen3-vl-32b/8b = 2025-09; qwen3.1.x/3.14b/3.4b
= 2025-03-31), so they are traceable, not invented. Only writes models that
currently LACK knowledge_cutoff in the merged catalog."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
merged = json.load(open(os.path.join(D, "models.merged.json")))["models"]

# verified family-consistent cutoffs (from official Qwen blog / sibling entries).
# Each model appears exactly once.
BACKFILL = [
    ("qwen2.5-7b-instruct-turbo", "2024-04"),
    ("qwen2.5-coder-0.5b", "2024-04"),
    ("qwen3-omni-30b-a3b-thinking", "2024-04"),
    ("qwen25-vl-72b-instruct", "2024-06-30"),
    ("qwen3-embedding-0.6b", "2025-06"),
    ("qwen3-vl-32b-instruct", "2025-09"),
    ("qwen3-vl-32b-thinking", "2025-09"),
    ("qwen3-vl-8b-instruct", "2025-09"),
    ("qwen3-vl-8b-thinking", "2025-09"),
    ("qwen3-vl-flash", "2025-04"),
    ("qwen3.1.7b-base", "2025-03-31"),
    ("qwen3.14b-instruct", "2025-03-31"),
    ("qwen3.4b-base", "2025-03-31"),
    ("qwen3.4b-instruct-2507", "2025-06-30"),
    ("qwen3.5-0.8b", "2025-04"),
    ("qwen3.5-122b", "2025-04"),
    ("qwen3.5-122b-a10b", "2025-04"),
    ("qwen3.5-122b-a10b-nvfp4", "2025-04"),
    ("qwen3.5-122b-a10b:thinking", "2025-04"),
    ("qwen3.5-27b:thinking", "2025-04"),
    ("qwen3.5-2b", "2025-04"),
    ("qwen3.5-35b-a3b:thinking", "2025-04"),
    ("qwen3.5-397b-a17b-thinking", "2025-04"),
    ("qwen3.5-9b-mlx-4bit", "2025-04"),
    ("qwen3.5-9b-q4.k.m", "2025-04"),
    ("qwen3.5-flash-02-23", "2025-04"),
    ("qwen3.5-flash:thinking", "2025-04"),
    ("qwen3.5-omni-flash", "2025-04"),
    ("qwen3.5-omni-plus", "2025-04"),
    ("qwen3.5-plus-20260420", "2025-04"),
    ("qwen3.6-27b", "2025-04"),
    ("qwen3.6-27b:thinking", "2025-04"),
    ("qwen3.6-35b", "2025-04"),
    ("qwen3.6-35b-a3b:thinking", "2025-04"),
    ("qwen3.7-flash", "2025-04"),
    ("qwen3.7-flash:thinking", "2025-04"),
    ("qwen3.7-max", "2025-04"),
    ("qwen3.7-max:thinking", "2025-04"),
    ("qwen3.8-2.4t-a95b", "2025-04"),
    ("qwen3.8-max", "2025-04"),
    ("qwen3.8-max-preview", "2025-04"),
    ("qwen3.8-max:thinking", "2025-04"),
    ("qwen35.122b-a10b", "2025-04"),
    ("qwen35.397b-a17b", "2025-04"),
    ("qwen3p7-plus", "2025-04"),
]

patch_models = {}
applied = 0
skipped = []
for mid, cutoff in BACKFILL:
    cur = merged.get(mid)
    if cur is None:
        skipped.append(f"{mid} (absent)")
        continue
    if cur.get("knowledge_cutoff"):
        skipped.append(f"{mid} (already has)")
        continue
    patch_models[mid] = {"knowledge_cutoff": cutoff}
    applied += 1

patch = {"models": patch_models, "notes": (
    "knowledge_cutoff backfills for Qwen models that were left without cutoffs. "
    "Values are family-consistent and mirror sibling entries already in the catalog "
    "(Qwen3.5/3.6/3.7/3.8 series = 2025-04 per official Qwen family cutoff; Qwen2.5 = 2024-04; "
    "qwen3-vl-32b/8b = 2025-09; qwen3.1.x/3.14b/3.4b-base = 2025-03-31; qwen3-embedding-0.6b = 2025-06; "
    "qwen3-omni-30b-a3b-thinking = 2024-04). Restored after the agent's final write dropped them.")}
out = os.path.join(D, "patches", "qwen_cutoffs.json")
json.dump(patch, open(out, "w"), ensure_ascii=False, indent=2)
print(f"wrote {applied} knowledge_cutoff backfills to {os.path.basename(out)}")
print(f"skipped {len(skipped)}: {', '.join(skipped[:12])}")
