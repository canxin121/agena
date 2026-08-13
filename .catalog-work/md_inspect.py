#!/usr/bin/env python3
"""Inspect structure of the local models.dev live json."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
for p in (os.path.join(D, "research", "models.dev.live.json"),
          os.path.join(D, "research", "models.dev.json")):
    if os.path.exists(p):
        print("FILE", p, os.path.getsize(p))
        j = json.load(open(p))
        print("type", type(j).__name__)
        if isinstance(j, dict):
            print("top keys:", list(j.keys())[:10])
            for k in list(j.keys())[:3]:
                print("  sample", k, "->", str(j[k])[:200])
        elif isinstance(j, list):
            print("len", len(j))
            print("sample", str(j[0])[:300])
        break
