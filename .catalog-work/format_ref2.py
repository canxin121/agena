#!/usr/bin/env python3
"""Find entries with speed_modes and any request_override/adapter_overrides patterns."""
import json
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(D + "/models.merged.json"))["models"]

with_sm = [m for m in cat.values() if m.get("speed_modes")]
print("entries with speed_modes:", len(with_sm))
for m in with_sm[:12]:
    print("="*70)
    print("id:", m.get("id"), "| origin:", m.get("origin"))
    print(json.dumps(m.get("speed_modes"), indent=1)[:1200])

# any thinking modes with request_override?
print("\n\n=== thinking modes containing request_override/adapter_overrides ===")
n = 0
for m in cat.values():
    tm = m.get("thinking_modes") or {}
    for k, v in tm.items():
        if isinstance(v, dict) and (v.get("request_override") or v.get("adapter_overrides")):
            n += 1
            if n <= 6:
                print("id:", m.get("id"), "| origin:", m.get("origin"))
                print(json.dumps({k: v}, indent=1)[:1200])
print("total with override:", n)
