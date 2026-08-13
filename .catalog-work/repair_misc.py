#!/usr/bin/env python3
"""Repair misc_long_tail.json patch to match models.dev reasoning_options.
Rules (data-derived, never invented):
- MISMATCH: patch pattern contradicts ropts kind -> rewrite to ropts.
- ropts empty -> drop thinking_modes (no data supports the template).
- ABSENT from models.dev but name-conclusive (:thinking / -think) -> always-on high.
- ABSENT with no name evidence or family evidence -> drop.
- Keep family-verified entries (Qwen3.5+ toggle, Trinity, Fugu siblings, Inkling research).
"""
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
PF = os.path.join(D, "patches/misc_long_tail.json")
patch = json.load(open(PF))
models = patch["models"]

md = json.load(open(os.path.join(D, "research/models.dev.json")))
dev = {}
for prov in md.values():
    for mid, m in (prov.get("models") or {}).items():
        real = m.get("id") or mid
        dev.setdefault(real, m); dev.setdefault(mid, m)

def ropts_kind(ro):
    if not ro: return None
    for r in ro:
        t = r.get("type")
        if t == "effort": return "effort"
        if t == "toggle": return "toggle"
        if t == "token": return "budget"
    return "other"

# --- reusable mode dicts ---
OFF = {"display_name": "Off", "strategy": "disabled"}
def E(lvl):
    return {"display_name": f"Think {lvl.capitalize()}", "strategy": "effort", "effort": lvl}
TOGGLE = {"off": OFF,
          "on": {"display_name": "Thinking", "strategy": "request_only",
                 "request_override": {"body_patch": {"thinking": True}}}}
ALWAYS_HIGH = {"high": E("high")}
def EFFORT(levels):
    d = {"off": OFF}
    for l in levels:
        d[l] = E(l)
    return d

changes = []

def drop_tm(mid, why):
    if mid in models and models[mid].get("thinking_modes"):
        models[mid]["thinking_modes"] = None
        changes.append(f"DROP {mid}: {why}")
    elif mid in models:
        changes.append(f"already-empty {mid}: {why}")

def set_tm(mid, tm, why):
    if mid not in models:
        changes.append(f"ADD {mid}: {why}")
        models[mid] = {"thinking_modes": tm}
    else:
        models[mid]["thinking_modes"] = tm
        changes.append(f"SET {mid}: {why}")

# ---- ropts-empty drops (template unsupported by data) ----
for mid in ["nova-2-lite", "gemma4-31b", "fugu-ultra", "mercury-2", "coding-minimax-m2.7"]:
    drop_tm(mid, "ropts=[] in models.dev, no data supports template")

# ---- ABSENT from models.dev ----
# name-conclusive always-on lanes -> keep always-on high
for mid in ["gemma4-31b:thinking", "holo3-35b-a3b:thinking", "olmo-3-32b-think",
            "ling-3.0-flash:thinking", "ling-3.0-tiny:thinking",
            "inkling:thinking", "inkling-small:thinking"]:
    set_tm(mid, ALWAYS_HIGH, "ABSENT but :thinking/-think lane => always-on")

# family-verified keeps
set_tm("qwen35.122b-a10b", TOGGLE, "Qwen3.5 family enable_thinking (keep)")
set_tm("qwen35.397b-a17b", TOGGLE, "Qwen3.5 family enable_thinking (keep)")
set_tm("qwen3p7-plus", TOGGLE, "Qwen3.7 family enable_thinking (keep)")
set_tm("tim-qwen3.6-27b", TOGGLE, "Qwen3.6 family enable_thinking (keep)")
set_tm("trinity-large-thinking", EFFORT(["low", "medium", "high"]),
       "same model as arcee-trinity-large-thinking (ropts=effort)")
set_tm("fugu-ultra-v1.1", {"off": OFF, "high": E("high"), "xhigh": E("xhigh")},
       "sibling fugu-ultra-20260615 ropts=[high,xhigh] (drop fabricated max)")

# ABSENT with no data -> drop
for mid in ["smollm3-3b-base", "interfaze-beta", "diffusiongemma-26b-a4b-it",
            "dola-seed-2.0-pro", "namazu", "muse-glimmer-30b",
            "cogito-v2-1-671b", "cogito-v2.1-671b"]:
    drop_tm(mid, "ABSENT from models.dev, no verifiable source")

# ---- MISMATCH: rewrite to match ropts ----
set_tm("gemma4", EFFORT(["minimal", "low", "medium", "high"]),
       "ropts=effort[none,minimal,low,medium,high] (was toggle)")
set_tm("gemma4-26b", {"off": OFF, "high": E("high")},
       "ropts=effort[none,high] (was toggle)")
set_tm("hy3", TOGGLE, "ropts=[toggle,effort] primary toggle")
set_tm("hy3-preview", TOGGLE, "ropts=[toggle,effort] primary toggle")
set_tm("nova-2-lite-v1", TOGGLE, "ropts=[toggle,effort] primary toggle")

json.dump(patch, open(PF, "w"), ensure_ascii=False, indent=2)
print(f"repaired: {len(changes)} changes")
for c in changes:
    print("  ", c)
