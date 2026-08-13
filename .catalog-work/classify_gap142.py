#!/usr/bin/env python3
"""Classify the 142 reasoning-gap models by what models.dev says about each:
ropts=toggle / effort / token -> real reasoning, likely needs thinking_modes
ropts absent -> feature flag likely wrong OR provider lacks reasoning_options
Also flags runtime-enriched IDs (deepseek-v4) and alias/router families."""
import json, os, sys
from collections import defaultdict

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
sys.path.insert(0, D)
from final_report import reasoning_supported

md = json.load(open(os.path.join(D, "research/models.dev.json")))
dev = {}
for prov in md.values():
    for mid, m in (prov.get("models") or {}).items():
        real = m.get("id") or mid
        dev.setdefault(real, m)
        dev.setdefault(mid, m)

cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
gap = {mid: x for mid, x in cat.items()
       if reasoning_supported(x) and not x.get("thinking_modes")}

def ropts_kind(ro):
    if not ro: return None
    for r in ro:
        t = r.get("type")
        if t == "effort": return "effort"
        if t == "toggle": return "toggle"
        if t == "token": return "budget"
    return "other"

# strip :thinking suffix to check base? also try direct
def lookup(mid):
    m = dev.get(mid)
    if m is not None:
        return mid, m
    # try without :suffix
    base = mid.split(":")[0]
    m = dev.get(base)
    if m is not None:
        return base, m
    return None, None

buckets = defaultdict(list)
runtime = []
for mid, x in sorted(gap.items()):
    if "deepseek-v4" in mid:
        runtime.append(mid); continue
    key, m = lookup(mid)
    if m is None:
        buckets["absent"].append(mid); continue
    kind = ropts_kind(m.get("reasoning_options"))
    if kind == "toggle":
        buckets["ropts-toggle"].append((mid, key))
    elif kind == "effort":
        buckets["ropts-effort"].append((mid, key))
    elif kind == "budget":
        buckets["ropts-budget"].append((mid, key))
    else:
        buckets["ropts-empty"].append(mid)

for b, rows in buckets.items():
    print(f"\n### {b} ({len(rows)})")
    for r in rows:
        if isinstance(r, tuple):
            mid, key = r
            print(f"  {mid}  (models.dev key: {key})")
        else:
            print(f"  {r}")
print(f"\n### runtime-enriched deepseek-v4 ({len(runtime)}): skip")
print("  " + ", ".join(runtime))
