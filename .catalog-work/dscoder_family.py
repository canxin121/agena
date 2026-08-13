#!/usr/bin/env python3
"""Check deepseek-coder family coverage + models.dev data for the base variants."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
md = json.load(open(os.path.join(D, "research", "models.dev.live.json")))

print("== catalog deepseek-coder / deepseek-coder variants ==")
for m in sorted(cat):
    if "coder" in m:
        print(f"  {m:52s} ctx={cat[m].get('context_window_tokens')} open={cat[m].get('open_weights')}")

print("\n== models.dev deepseek-coder entries ==")
for prov, pv in md.items():
    if not isinstance(pv, dict) or "models" not in pv:
        continue
    for mid, m in pv["models"].items():
        if "coder-6.7" in mid or "deepseek-coder" in mid.lower():
            print(f"  {mid:55s} prov={prov:16s} ctx={m.get('limit',{}).get('context')} rd={m.get('release_date')}")
