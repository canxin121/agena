#!/usr/bin/env python3
"""Dump reasoning_options for EVERY bundle model across all models.dev providers,
so the final patch is built from verified data only."""
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
bundle = json.load(open(os.path.join(D, "bundle_entries/alibaba.json")))
md = json.load(open(os.path.join(D, "modelsdev.json")))

def find(mid):
    exact = {}
    for prov, pd in md.items():
        if not isinstance(pd, dict):
            continue
        m = pd.get("models")
        if not isinstance(m, dict):
            continue
        if mid in m:
            exact[prov] = m[mid]
    # suffix match
    suffix = {}
    for prov, pd in md.items():
        if not isinstance(pd, dict):
            continue
        m = pd.get("models")
        if not isinstance(m, dict):
            continue
        for k, v in m.items():
            if k.split("/")[-1] == mid:
                suffix.setdefault(prov, v)
    return exact, suffix

for bid in sorted(bundle):
    ex, sf = find(bid)
    if not (ex or sf):
        print("=== %s === NOT FOUND in models.dev" % bid)
        continue
    for prov, v in {**ex, **{k: v for k, v in sf.items() if k not in ex}}.items():
        ro = v.get("reasoning_options")
        lim = v.get("limit") or {}
        know = v.get("knowledge")
        rel = v.get("release_date")
        print("=== %s [%s] ropts=%s ctx=%s out=%s know=%s rel=%s" % (
            bid, prov, json.dumps(ro), lim.get("context"), lim.get("output"), know, rel))
