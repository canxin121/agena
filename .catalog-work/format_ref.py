#!/usr/bin/env python3
"""Print reference thinking_modes/speed_modes from existing entries."""
import json
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(D + "/models.merged.json"))["models"]

with_tm = [m for m in cat.values() if m.get("thinking_modes")]
print("entries with thinking_modes:", len(with_tm))
seen_origin = {}
for m in with_tm:
    o = m.get("origin") or "?"
    if o not in seen_origin:
        seen_origin[o] = m
for o in ["Anthropic", "OpenAI", "Google", "DeepSeek", "Alibaba"]:
    m = seen_origin.get(o)
    if m:
        print("="*70)
        print("ORIGIN", o, "| id:", m.get("id"))
        print(json.dumps({"thinking_modes": m.get("thinking_modes"),
                          "speed_modes": m.get("speed_modes"),
                          "capabilities": m.get("capabilities"),
                          "pricing": m.get("pricing")}, indent=1)[:1800])
