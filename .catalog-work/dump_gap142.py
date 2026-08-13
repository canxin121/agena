#!/usr/bin/env python3
"""Dump the reasoning-gap list (models with reasoning supported but no
thinking_modes) with current metadata, grouped by origin, to gaps/
reasoning_gap_142.json + a readable text listing."""
import json, os, sys

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
sys.path.insert(0, D)
from final_report import reasoning_supported

cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
gap = {mid: x for mid, x in cat.items()
       if reasoning_supported(x) and not x.get("thinking_modes")}

out = os.path.join(D, "gaps", "reasoning_gap_142.json")
json.dump(gap, open(out, "w"), ensure_ascii=False, indent=2)
print(f"wrote {len(gap)} gap models -> {out}")

# readable triage listing, sorted by origin then id
rows = sorted(gap.items(), key=lambda kv: (kv[1].get("origin", "?"), kv[0]))
for mid, x in rows:
    d = x.get("description") or ""
    d = d[:60] if d else "NO-DESC"
    print(f"[{x.get('origin','?'):14}] {mid:44} ctx={x.get('context_window_tokens')} d={d}")
