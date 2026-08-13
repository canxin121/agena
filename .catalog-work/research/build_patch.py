import json

# ---- thinking mode shapes ----
# Shape A: DeepSeek models exposing the `thinking` request param (toggleable, thinking default)
SHAPE_A = {
    "off": {"display_name": "Off", "strategy": "disabled"},
    "on": {
        "display_name": "Thinking",
        "strategy": "request_only",
        "request_override": {"body_patch": {"thinking": True}},
    },
}
# Shape B: reasoning always-on (reasoning-only / :thinking lanes / r1) - no off toggle
SHAPE_B = {"on": {"display_name": "Thinking", "strategy": "request_only"}}
# StepFun 3.5 flash: always-on reasoning, effort low/high (no off)
STEP_35 = {
    "low": {"display_name": "Think Low", "strategy": "effort", "effort": "low"},
    "high": {"display_name": "Think High", "strategy": "effort", "effort": "high"},
}
# StepFun 3.7 flash: always-on reasoning, effort low/medium/high, default medium
STEP_37 = {
    "default": "medium",
    "low": {"display_name": "Think Low", "strategy": "effort", "effort": "low"},
    "medium": {"display_name": "Think Medium", "strategy": "effort", "effort": "medium"},
    "high": {"display_name": "Think High", "strategy": "effort", "effort": "high"},
}

models = {}

def add(mid, thinking=None, max_input=None, knowledge=None, pricing=None):
    m = {}
    if thinking is not None:
        m["thinking_modes"] = thinking
    if max_input is not None:
        m["max_input_tokens"] = max_input
    if knowledge is not None:
        m["knowledge_cutoff"] = knowledge
    if pricing is not None:
        m["pricing"] = pricing
    models[mid] = m

# ---- DeepSeek V4 family: thinking param toggle (Shape A) ----
V4A = [
    "alicloud-deepseek-v4-flash",
    "alicloud-deepseek-v4-pro",
    "deep-deepseek-v4-flash",
    "deep-deepseek-v4-pro",
    "deepseek-v4-flash-0731-fast",
    "deepseek-v4-flash-0731@eu",
    "deepseek-v4-flash-el",
    "deepseek-v4-pro-el",
    "deepseek-v4-flash-latest",
    "deepseek-latest",
    "deepseek-v4-flash:0731",
    "deepseek-v4-flash:discounted",
    "deepseek-v4-pro:discounted",
    "deepseek-v4-pro-lightning",
    "deepseek-v4-pro-0813",
    "umans-deepseek-v4-flash-0731",
]
for mid in V4A:
    add(mid, thinking=SHAPE_A)

# ---- DeepSeek 3.x hybrid base models: thinking param toggle (Shape A) ----
# deepseek-v3.2-fast: nebius lists reasoning_options [{type: toggle}] -> Shape A
for mid in ["deepseek-3.2", "deepseek-chat-v3.1", "deepseek-v3.2-251201", "deepseek-v3.2-nvfp4",
            "deepseek-v3.1-terminus", "deepseek-v3.2-fast"]:
    add(mid, thinking=SHAPE_A)

# ---- DeepSeek reasoning-only / always-on (Shape B) ----
for mid in [
    "deepseek-r1-0528-qwen3-8b",
    "deepseek-r1-turbo",
    "deepseek-math-v2",
    "deepseek-v3.2-exp-thinking",
    "deepseek-v3.2-thinking",
    "deepseek-v3.2:thinking",
    "deepseek-v4-flash-0731:thinking",
    "deepseek-v4-flash:thinking",
    "deepseek-v4-pro-0813:thinking",
    "deepseek-v4-pro:thinking",
]:
    add(mid, thinking=SHAPE_B)

# ---- StepFun ----
add("step-3-5-flash", thinking=STEP_35)
add("step-3-5-flash-2603", thinking=STEP_35)
add("step-3.5-flash", thinking=STEP_35)
add("step-3.5-flash-2603", thinking=STEP_35)
add("step-3-7-flash", thinking=STEP_37)
add("step-3.7-flash", thinking=STEP_37)
add("step-3.7-flash:thinking", thinking=SHAPE_B)

# ---- max_input_tokens (verified context == max input for these; models.dev limit.input where given) ----
MAX_IN = {
    "alicloud-deepseek-v4-flash": 1000000,
    "alicloud-deepseek-v4-pro": 1000000,
    "deep-deepseek-v4-flash": 1000000,
    "deep-deepseek-v4-pro": 1000000,
    "deepseek-3.2": 163840,
    "deepseek-chat-v3.1": 163840,
    "deepseek-math-v2": 160000,
    "deepseek-r1-0528-qwen3-8b": 128000,
    "deepseek-r1-turbo": 64000,
    "deepseek-v3.2-251201": 128000,
    "deepseek-v3.2-nvfp4": 131072,
    "deepseek-v3.2-thinking": 128000,
    "deepseek-v4-flash-0731-fast": 1000000,
    "deepseek-v4-flash-0731@eu": 1048576,
    "deepseek-v4-flash:0731": 1048576,
    "deepseek-v4-flash:discounted": 1048576,
    "deepseek-v4-pro-lightning": 1000000,
    "deepseek-v4-pro:discounted": 1048576,
    "umans-deepseek-v4-flash-0731": 1048576,
}
for mid, v in MAX_IN.items():
    models[mid]["max_input_tokens"] = v

# ---- knowledge_cutoff (base-model mapping for hosted/alias v4 models; direct where models.dev gives it) ----
KNO = {
    "deepseek-v4-flash-el": "2025-05",
    "deepseek-v4-pro-el": "2025-05",
    "deepseek-v4-flash-latest": "2025-05",
    "deepseek-v4-flash:discounted": "2025-05",
    "deepseek-v4-pro:discounted": "2025-05",
    "deepseek-v4-pro-0813": "2025-05",
    "deepseek-v4-pro-0813:thinking": "2025-05",
    "deepseek-v4-pro-lightning": "2025-05",
}
for mid, v in KNO.items():
    models[mid]["knowledge_cutoff"] = v

# ---- pricing (official DeepSeek V4 Flash 0731 price; bundle lacks for this model) ----
models["deepseek-v4-flash:0731"]["pricing"] = {
    "input_usd_per_million_tokens": "0.14",
    "output_usd_per_million_tokens": "0.28",
    "cache_read_usd_per_million_tokens": "0.0028",
}

# ---- verify all 39 bundle models are covered ----
bundle = json.load(open('/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/bundle_entries/deepseek.json'))
bm = set(bundle.keys())
pm = set(models.keys())
print("bundle models:", len(bm), "patch models:", len(pm))
print("in bundle but NOT patched:", sorted(bm - pm))
print("patched but NOT in bundle:", sorted(pm - bm))

patch = {
    "models": dict(sorted(models.items())),
    "notes": (
        "DeepSeek thinking: official api-docs.deepseek.com thinking-mode guide + pricing page confirm "
        "deepseek-v4-flash/v4-pro (and the V4/3.2 hybrid families) support thinking and non-thinking modes "
        "via a request-time `thinking` param (thinking on by default); modeled as request_only 'on' with "
        "off disabled. Reasoning-only lanes (deepseek-r1 variants, deepseek-math-v2, the exp-thinking / "
        ":thinking / -thinking variants of V3.2 and V4) are always-on reasoning, modeled with no off toggle. "
        "StepFun (step-3.5/3.7-flash) reasoning is always-on with reasoning_effort low/medium/high "
        "(default medium per platform.stepfun.com docs), modeled as effort modes with no off. "
        "NO speed_modes added: neither DeepSeek nor StepFun documents a speed/service-tier toggle "
        "(the '-fast'/'el' suffixes are distinct model ids, not per-request speed modes). "
        "max_input_tokens set = verified context window where the bundle lacked it. knowledge_cutoff set "
        "via base-model mapping for hosted/alias V4 models (base V4 knowledge 2025-05 per models.dev). "
        "Could NOT verify: knowledge_cutoff for deepseek-latest, deepseek-math-v2, deepseek-r1-0528-qwen3-8b, "
        "deepseek-r1-turbo, deepseek-v3.2-251201, deepseek-v3.2-exp-thinking (no source value); pricing for "
        "deepseek-math-v2 and deepseek-v3.2-251201 (no provider lists cost)."
    ),
}

out = '/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work/patches/deepseek.json'
with open(out, 'w') as f:
    json.dump(patch, f, indent=2, ensure_ascii=False)
    f.write("\n")
print("wrote", out)
