#!/usr/bin/env python3
"""Repair alibaba patch mismatches against models.dev reasoning_options."""
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
PF = os.path.join(D, "patches/alibaba.json")
patch = json.load(open(PF))
models = patch["models"]

md = json.load(open(os.path.join(D, "research/models.dev.json")))
dev = {}
for prov in md.values():
    for mid, m in (prov.get("models") or {}).items():
        real = m.get("id") or mid
        dev.setdefault(real, m); dev.setdefault(mid, m)

OFF = {"display_name": "Off", "strategy": "disabled"}
def E(lvl):
    return {"display_name": f"Think {lvl.capitalize()}", "strategy": "effort", "effort": lvl}
TOGGLE = {"off": OFF,
          "on": {"display_name": "Thinking", "strategy": "request_only",
                 "request_override": {"body_patch": {"enable_thinking": True}}}}
def EFFORT(levels):
    d = {"off": OFF}
    for l in levels:
        d[l] = E(l)
    return d

changes = []

def set_tm(mid, tm, why):
    models[mid]["thinking_modes"] = tm
    changes.append(f"SET {mid}: {why}")

# From verify_alibaba_patch.py mismatches (patch contradicts models.dev ropts):
set_tm("alibaba-qwen3-32b", EFFORT(["low", "medium", "high", "max"]),
       "ropts=effort[none,low,medium,high,max]")
set_tm("deepseek-v3.2", EFFORT(["low", "medium", "high"]),
       "ropts=effort[none,low,medium,high]")
set_tm("qwen3.5-397b-a17b", EFFORT(["minimal", "low", "medium", "high"]),
       "ropts=effort[none,minimal,low,medium,high]")
set_tm("qwen3.5-9b", EFFORT(["low", "medium", "high"]),
       "ropts=effort[none,low,medium,high]")
set_tm("qwen3.6-27b", EFFORT(["low", "medium", "high"]),
       "ropts=effort[none,low,medium,high]")
set_tm("qwen3.6-35b", {"off": OFF, "high": E("high")},
       "ropts=effort[none,high]")
set_tm("qwen3.6-35b-a3b", EFFORT(["minimal", "low", "medium", "high"]),
       "ropts=effort[none,minimal,low,medium,high]")
set_tm("qwen3.7-flash", {"off": OFF, "high": E("high")},
       "ropts=effort[none,high]")
set_tm("qwen3.8-max", TOGGLE, "ropts=[toggle,budget_tokens max 262144]")

json.dump(patch, open(PF, "w"), ensure_ascii=False, indent=2)
print(f"repaired {len(changes)} mismatches:")
for c in changes:
    print("  ", c)
