#!/usr/bin/env python3
"""Analyze how much of the missing catalog metadata can be authoritatively
backfilled from models.dev per-model fields (knowledge/cost/limit/reasoning_options).
Does NOT write yet — reports coverage."""
import json, os, sys
from collections import defaultdict

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
md = json.load(open(os.path.join(D, "research/models.dev.live.json")))
dev = {}
for prov in md.values():
    for mid, m in (prov.get("models") or {}).items():
        real = m.get("id") or mid
        dev.setdefault(real, m)
        dev.setdefault(mid, m)

cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

def count(field):
    return sum(1 for x in cat.values() if x.get(field) is None)

# For each missing field, how many have models.dev data?
def backfillable(field, src):
    n = 0
    for mid, x in cat.items():
        if x.get(field) is not None:
            continue
        m = dev.get(mid)
        if m is None:
            m = dev.get(mid.split(":")[0])
        if m is None:
            continue
        if m.get(src):
            n += 1
    return n

print("=== missing in catalog vs models.dev availability ===")
print(f"knowledge_cutoff missing: {count('knowledge_cutoff')}, models.dev has knowledge: {backfillable('knowledge_cutoff','knowledge')}")
print(f"pricing missing: {count('pricing')}, models.dev has cost: {backfillable('pricing','cost')}")
print(f"context_window_tokens missing: {count('context_window_tokens')}, models.dev has limit.context: {backfillable('context_window_tokens','limit')}")
print(f"max_output_tokens missing: {count('max_output_tokens')}, models.dev has limit.output: {backfillable('max_output_tokens','limit')}")
print(f"max_input_tokens missing: {count('max_input_tokens')}, models.dev has limit.input: {backfillable('max_input_tokens','limit')}")

# reasoning_options coverage: models with non-empty ropts that lack thinking_modes
def ropts_nonempty(ro):
    return isinstance(ro, list) and len(ro) > 0

no_tm_but_ropts = []
for mid, x in cat.items():
    if x.get("thinking_modes"):
        continue
    m = dev.get(mid) or dev.get(mid.split(":")[0])
    if m and ropts_nonempty(m.get("reasoning_options")):
        no_tm_but_ropts.append((mid, m["reasoning_options"]))
print(f"\nmodels with NO thinking_modes but models.dev ropts non-empty: {len(no_tm_but_ropts)}")
for mid, ro in sorted(no_tm_but_ropts)[:60]:
    print(f"  {mid}: {json.dumps(ro)}")
