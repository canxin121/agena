#!/usr/bin/env python3
"""Check last_updated convention for deepseek + kimi siblings."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

print("== deepseek last_updated examples ==")
n = 0
for m, e in sorted(cat.items()):
    if m.startswith("deepseek") and e.get("last_updated") and n < 8:
        print(f"  {m:40s} release={e.get('release_date')} last={e['last_updated']}")
        n += 1

print("\n== kimi last_updated examples ==")
n = 0
for m, e in sorted(cat.items()):
    if m.startswith("kimi") and e.get("last_updated") and n < 8:
        print(f"  {m:40s} release={e.get('release_date')} last={e['last_updated']} open={e.get('open_weights')}")
        n += 1
