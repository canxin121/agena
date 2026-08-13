#!/usr/bin/env python3
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
md = json.load(open(os.path.join(D, "modelsdev.json")))
targets = ["qwen-3-8-max", "qwen3.5-122b", "qwen3.5-122b-a10b", "qwen3.6-max", "qwen3.8-max-preview", "qwen3.8-max", "qwen3.8-2.4t-a95b", "qwen3.6-max-preview", "alibaba-qwen3-32b", "qwen3.5-9b"]
for t in targets:
    print("=== %s ===" % t)
    for prov, pd in md.items():
        if not isinstance(pd, dict):
            continue
        m = pd.get("models")
        if not isinstance(m, dict):
            continue
        for k, v in m.items():
            if k == t or k.split("/")[-1] == t:
                lim = v.get("limit") or {}
                print("  [%s] %s | ctx=%s out=%s | ropts=%s | know=%s | rel=%s | name=%s" % (
                    prov, k, lim.get("context"), lim.get("output"),
                    json.dumps(v.get("reasoning_options")), v.get("knowledge"),
                    v.get("release_date"), v.get("name")))
