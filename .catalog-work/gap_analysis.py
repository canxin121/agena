#!/usr/bin/env python3
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
bundle = json.load(open(os.path.join(D, "bundle_entries/alibaba.json")))
md = json.load(open(os.path.join(D, "modelsdev.json")))
bids = list(bundle.keys())

def find_dev(mid):
    cands = []
    for prov, pd in md.items():
        if not isinstance(pd, dict):
            continue
        m = pd.get("models")
        if not isinstance(m, dict):
            continue
        if mid in m:
            cands.append((prov, m[mid]))
    if cands:
        return cands
    for prov, pd in md.items():
        if not isinstance(pd, dict):
            continue
        m = pd.get("models")
        if not isinstance(m, dict):
            continue
        for k, v in m.items():
            if k.split("/")[-1].lower() == mid.lower():
                cands.append((prov, v))
    return cands

missing_fields = ["context_window_tokens", "max_input_tokens", "max_output_tokens", "knowledge_cutoff", "description", "pricing"]
for bid in bids:
    cur = bundle[bid]
    miss = [f for f in missing_fields if cur.get(f) in (None, {})]
    if not miss:
        continue
    rows = find_dev(bid)
    best = None
    for prov, m in rows:
        if prov in ("alibaba", "alibaba-cn"):
            best = m
            break
    if best is None and rows:
        best = rows[0][1]
    devctx = devout = devknow = devcost = devdesc = None
    if best:
        lim = best.get("limit") or {}
        devctx = lim.get("context")
        devout = lim.get("output")
        devknow = best.get("knowledge")
        devcost = best.get("cost")
        devdesc = best.get("description")
    flags = []
    if "context_window_tokens" in miss and devctx:
        flags.append("ctx=%s" % devctx)
    if "max_input_tokens" in miss and devctx:
        flags.append("maxin=%s" % devctx)
    if "max_output_tokens" in miss and devout:
        flags.append("maxout=%s" % devout)
    if "knowledge_cutoff" in miss and devknow:
        flags.append("know=%s" % devknow)
    if "pricing" in miss and devcost:
        flags.append("pricing=Y")
    if "description" in miss and devdesc:
        flags.append("desc=Y")
    if flags:
        print("%s: %s" % (bid, "; ".join(flags)))
