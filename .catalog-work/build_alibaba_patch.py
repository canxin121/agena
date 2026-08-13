#!/usr/bin/env python3
"""Build patches/alibaba.json — thinking/speed modes + max_input_tokens for the
91 models in bundle_entries/alibaba.json, from verified models.dev data
(alibaba/alibaba-cn providers preferred) + official DashScope/Qwen/Zhipu/MiniMax docs.

Categories:
  TOGGLE_QWEN  -> Qwen3-family hybrid: enable_thinking request toggle (off disabled + on request_only)
  TOGGLE_DS    -> DeepSeek V3.1/V3.2 hybrid: `thinking` bool toggle
  TOGGLE_GLM   -> GLM-5/5.1 on Alibaba: enable_thinking toggle (matches alicloud-glm-5.1)
  EFFORT_Q38   -> Qwen3.8 family: reasoning_effort low/medium/xhigh, default xhigh, off disabled
  EFFORT_GLM52 -> GLM-5.2: effort off/high/max, default max (matches zhipu patch convention)
  ALWAYS_ON    -> always-thinking (QwQ, DeepSeek-R1, all -thinking variants, MiniMax M2.x)
  NONE         -> non-thinking (base/Instruct-non-reasoning) -> omit thinking_modes
  SKIP         -> deepseek-v4 family: runtime enriches via openai_compatible_reasoning_modes
"""
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
bundle = json.load(open(os.path.join(D, "bundle_entries/alibaba.json")))
md = json.load(open(os.path.join(D, "modelsdev.json")))

ALWAYS_ON = {"high": {"display_name": "Think High", "strategy": "effort", "effort": "high"}}

def toggle_qwen():
    return {
        "off": {"display_name": "Off", "strategy": "disabled"},
        "on": {"display_name": "Thinking", "strategy": "request_only",
               "request_override": {"body_patch": {"enable_thinking": True}}},
    }

def toggle_ds():
    return {
        "off": {"display_name": "Off", "strategy": "disabled"},
        "on": {"display_name": "Thinking", "strategy": "request_only",
               "request_override": {"body_patch": {"thinking": True}}},
    }

def toggle_glm():
    return {
        "off": {"display_name": "Off", "strategy": "disabled"},
        "on": {"display_name": "Thinking", "strategy": "request_only",
               "request_override": {"body_patch": {"enable_thinking": True}}},
    }

def effort_q38():
    return {
        "default": "xhigh",
        "off": {"display_name": "Off", "strategy": "disabled"},
        "low": {"display_name": "Think Low", "strategy": "effort", "effort": "low"},
        "medium": {"display_name": "Think Medium", "strategy": "effort", "effort": "medium"},
        "xhigh": {"display_name": "Think Extra-High", "strategy": "effort", "effort": "xhigh"},
    }

def effort_glm52():
    return {
        "default": "max",
        "off": {"display_name": "Off", "strategy": "disabled"},
        "high": {"display_name": "Think High", "strategy": "effort", "effort": "high"},
        "max": {"display_name": "Think Max", "strategy": "effort", "effort": "max"},
    }

TOGGLE_QWEN = [
    "alibaba-qwen3-32b",
    "qwen-3-14b", "qwen-3-235b", "qwen-3-30b", "qwen-3-32b",
    "qwen-3-6-plus", "qwen-3-7-max", "qwen-3-7-plus", "qwen-3.6-max-preview",
    "qwen-flash", "qwen-plus",
    "qwen3-omni-flash", "qwen3-vl-plus",
    "qwen3.14b", "qwen3.30b-a3b", "qwen3.32b", "qwen3.235b", "qwen3.235b-a22b",
    "qwen3.8b", "qwen3.4b-instruct-2507",
    "qwen3.5-9b", "qwen3.5-122b", "qwen3.5-122b-a10b-nvfp4",
    "qwen3.5-397b-a17b", "qwen3.5-397b-a17b-fast",
    "qwen3.5-flash-02-23", "qwen3.5-plus", "qwen3.5-plus-02-15", "qwen3.5-plus-20260420",
    "qwen3.6-27b", "qwen3.6-35b", "qwen3.6-35b-a3b", "qwen3.6-flash",
    "qwen3.6-max", "qwen3.6-plus",
    "qwen3.7-flash", "qwen3.7-max", "qwen3.7-plus",
]
TOGGLE_DS = ["deepseek-v3.1", "deepseek-v3.2", "deepseek-v3.2-exp"]
TOGGLE_GLM = ["glm-5", "glm-5.1"]
EFFORT_Q38 = ["qwen-3-8-max", "qwen3.8-2.4t-a95b", "qwen3.8-max", "qwen3.8-max-preview"]
EFFORT_GLM52 = ["glm-5.2"]
ALWAYS_ON_MODELS = [
    "deepseek-r1", "deepseek-r1-0528",
    "deepseek-r1-distill-llama-70b", "deepseek-r1-distill-llama-8b",
    "deepseek-r1-distill-qwen-1-5b", "deepseek-r1-distill-qwen-14b",
    "deepseek-r1-distill-qwen-32b", "deepseek-r1-distill-qwen-7b",
    "qwq-32b", "qwq-plus",
    "minimax-m2.5",
    "qwen-plus-2025-07-28:thinking",
    "qwen3-max-thinking",
    "qwen3-next-80b-a3b-thinking", "qwen3-next-80b-a3b-thinking-fast",
    "qwen3-omni-30b-a3b-thinking",
    "qwen3-vl-235b-a22b-thinking", "qwen3-vl-32b-thinking",
    "qwen3-vl-8b-thinking", "qwen3-vl-thinking",
    "qwen3.235b-a22b-thinking", "qwen3.235b-a22b-thinking-2507",
    "qwen3.235b-a22b-thinking-2507-fast", "qwen3.30b-a3b-thinking-2507",
    "qwen3.5-122b-a10b:thinking", "qwen3.5-27b:thinking", "qwen3.5-35b-a3b:thinking",
    "qwen3.5-397b-a17b-thinking", "qwen3.5-flash:thinking", "qwen3.5-plus-thinking",
    "qwen3.6-27b:thinking", "qwen3.6-35b-a3b:thinking",
    "qwen3.7-flash:thinking", "qwen3.7-max:thinking", "qwen3.7-plus:thinking",
    "qwen3.8-max:thinking",
]

PLAN = {}
for m in TOGGLE_QWEN: PLAN[m] = ("toggle_qwen",)
for m in TOGGLE_DS: PLAN[m] = ("toggle_ds",)
for m in TOGGLE_GLM: PLAN[m] = ("toggle_glm",)
for m in EFFORT_Q38: PLAN[m] = ("effort_q38",)
for m in EFFORT_GLM52: PLAN[m] = ("effort_glm52",)
for m in ALWAYS_ON_MODELS: PLAN[m] = ("always_on",)

def dev_context_and_know(mid):
    """Return (context, knowledge) preferring alibaba/alibaba-cn, else first provider.
    Matches bare keys and provider-prefixed keys (suffix fallback)."""
    best_ctx = best_know = None
    found = False
    for prov, pd in md.items():
        if not isinstance(pd, dict):
            continue
        m = pd.get("models")
        if not isinstance(m, dict):
            continue
        key = mid if mid in m else None
        if key is None:
            for k in m:
                if k.split("/")[-1] == mid:
                    key = k
                    break
        if key is None:
            continue
        lim = m[key].get("limit") or {}
        ctx = lim.get("context")
        know = m[key].get("knowledge")
        if prov in ("alibaba", "alibaba-cn"):
            return ctx, know
        if not found:
            best_ctx, best_know = ctx, know
            found = True
    return best_ctx, best_know

import re
def clean_know(k):
    if not k:
        return None
    m = re.match(r"^(\d{4}-\d{2})", k)
    return m.group(1) if m else None

models = {}
filled_maxin = 0
filled_know = 0
skip_maxin = []
for bid in sorted(bundle):
    entry = {}
    if bid in PLAN:
        cat = PLAN[bid][0]
        entry["thinking_modes"] = {
            "toggle_qwen": toggle_qwen(), "toggle_ds": toggle_ds(),
            "toggle_glm": toggle_glm(), "effort_q38": effort_q38(),
            "effort_glm52": effort_glm52(), "always_on": ALWAYS_ON,
        }[cat]
    cur = bundle[bid]
    if cur.get("max_input_tokens") in (None,):
        ctx, know = dev_context_and_know(bid)
        bctx = cur.get("context_window_tokens")
        if ctx and ctx == bctx:
            entry["max_input_tokens"] = ctx
            filled_maxin += 1
        else:
            skip_maxin.append((bid, ctx, bctx))
    if cur.get("knowledge_cutoff") in (None,):
        ctx, know = dev_context_and_know(bid)
        ck = clean_know(know)
        if ck:
            entry["knowledge_cutoff"] = ck
            filled_know += 1
    if entry:
        models[bid] = entry

patch = {
    "models": models,
    "notes": "PLACEHOLDER",
}
out = os.path.join(D, "patches", "alibaba.json")
with open(out, "w") as f:
    json.dump(patch, f, ensure_ascii=False, indent=2)
print("wrote", out, "with", len(models), "models")
print("filled max_input_tokens:", filled_maxin, "| filled knowledge_cutoff:", filled_know)
print("max_input NOT filled (id, devctx, bundlectx):")
for r in skip_maxin:
    print("  ", r)
