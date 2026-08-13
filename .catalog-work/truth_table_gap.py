#!/usr/bin/env python3
"""Build a truth table for the 142 reasoning-gap models:
merged features.reasoning vs models.dev authoritative `reasoning` + reasoning_options.
Output buckets:
  F            -> merged says reasoning but models.dev reasoning=false  (flag bug)
  REAL-ROPT    -> reasoning=true + non-empty ropts (add thinking_modes)
  REAL-ALWAYS  -> reasoning=true + empty/absent ropts (always-on, no options)
  NO-MD        -> absent from models.dev (needs other evidence)
  DEEPSEEK     -> runtime-enriched, skip
"""
import json, os, sys
from collections import defaultdict

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
sys.path.insert(0, D)
from final_report import reasoning_supported

md = json.load(open(os.path.join(D, "research/models.dev.live.json")))
dev = {}
for prov in md.values():
    for mid, m in (prov.get("models") or {}).items():
        real = m.get("id") or mid
        dev.setdefault(real, m)
        dev.setdefault(mid, m)

cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
gap = {mid: x for mid, x in cat.items()
       if reasoning_supported(x) and not x.get("thinking_modes")}

def md_lookup(mid):
    m = dev.get(mid)
    if m is not None:
        return mid, m
    base = mid.split(":")[0]
    m = dev.get(base)
    if m is not None:
        return base, m
    return None, None

def ropts_nonempty(ro):
    return isinstance(ro, list) and len(ro) > 0

buckets = defaultdict(list)
for mid in sorted(gap):
    if "deepseek-v4" in mid:
        buckets["DEEPSEEK(runtime,skip)"].append(mid); continue
    key, m = md_lookup(mid)
    if m is None:
        buckets["NO-MD"].append(mid); continue
    reasoning = bool(m.get("reasoning"))
    ro = m.get("reasoning_options")
    if not reasoning:
        buckets["FALSE-FLAG(remove reasoning)"].append((mid, key))
    elif ropts_nonempty(ro):
        buckets["REAL-ROPT(add modes)"].append((mid, key, ro))
    else:
        buckets["REAL-ALWAYS(empty ropts)"].append((mid, key))

for b, rows in sorted(buckets.items()):
    print(f"\n### {b} ({len(rows)})")
    for r in rows:
        if isinstance(r, tuple) and len(r) == 3:
            mid, key, ro = r
            print(f"  {mid}  (md:{key}) ropts={json.dumps(ro)}")
        elif isinstance(r, tuple):
            mid, key = r
            print(f"  {mid}  (md:{key})")
        else:
            print(f"  {r}")
