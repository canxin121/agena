#!/usr/bin/env python3
"""Generate thinking_modes for the 18 REAL-ROPT models (models.dev confirms
reasoning + non-empty reasoning_options). Maps models.dev ropts to the catalog's
canonical shapes:
  - ropts effort values include 'none'   -> off(disabled) + effort ladder
  - ropts has toggle + effort ladder     -> off(disabled) + effort ladder (toggle==off)
  - ropts pure effort                    -> effort ladder only (+ default)
Uses the same display_name/description conventions as existing catalog entries.
Writes patches/ropt_modes.json (apply_patches.py: null deletes, so omit-none)."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
md = json.load(open(os.path.join(D, "research/models.dev.live.json")))
dev = {}
for prov in md.values():
    for mid, m in (prov.get("models") or {}).items():
        real = m.get("id") or mid
        dev.setdefault(real, m)
        dev.setdefault(mid, m)

EFFORT_DESC = {
    "low": ("Think Low", "Fast responses with lighter reasoning"),
    "medium": ("Think Medium", "Balances speed and reasoning depth for everyday tasks"),
    "high": ("Think High", "Greater reasoning depth for complex problems"),
    "xhigh": ("Think X-High", "Maximum reasoning depth for the hardest problems"),
    "max": ("Think Max", "Extreme reasoning depth for frontier-scale problems"),
}

# model -> ropts (from models.dev, verified non-empty)
TARGETS = {
    "agent-max": [{"type": "toggle"}, {"type": "effort", "values": ["low", "medium", "high", "xhigh", "max"]}],
    "agent-prime": [{"type": "effort", "values": ["low", "medium", "high"]}],
    "agent-standard": [{"type": "effort", "values": ["low", "medium", "high"]}],
    "cheap": [{"type": "effort", "values": ["none", "low", "medium", "high"]}],
    "code-max": [{"type": "toggle"}, {"type": "effort", "values": ["low", "medium", "high", "xhigh", "max"]}],
    "code-prime": [{"type": "effort", "values": ["low", "medium", "high"]}],
    "code-standard": [{"type": "effort", "values": ["low", "medium", "high"]}],
    "e2e": [{"type": "effort", "values": ["none", "low", "medium", "high"]}],
    "green-r": [{"type": "effort", "values": ["low", "medium", "high"]}],
    "nemotron-3-super-120b-a12b:thinking": [{"type": "effort", "values": ["low", "medium"]}],
    "nemotron-3-ultra-550b-a55b:thinking": [{"type": "effort", "values": ["medium", "high"]}],
    "nemotron-nano-12b-v2-vl": [{"type": "effort", "values": ["none", "low", "medium", "high", "max"]}],
    "openai-gpt-4o-mini": [{"type": "effort", "values": ["low", "medium", "high"]}],
    "synth": [{"type": "effort", "values": ["none", "low", "medium", "high"]}],
    "synth-code": [{"type": "effort", "values": ["none", "low", "medium", "high"]}],
    "text-max": [{"type": "toggle"}, {"type": "effort", "values": ["low", "medium", "high", "xhigh", "max"]}],
    "text-prime": [{"type": "effort", "values": ["low", "medium", "high"]}],
    "text-standard": [{"type": "effort", "values": ["low", "medium", "high"]}],
}

def build_modes(ropts, is_thinking_variant=False):
    """Build thinking_modes dict from models.dev ropts list.

    - `:thinking` variants are always-on reasoning (no off); keep a single
      effort-high mode per catalog convention.
    - effort values including 'none' -> off(disabled) + effort ladder.
    - toggle + effort ladder -> off(disabled) + effort ladder.
    - pure effort ladder -> ladder only (+ default).
    """
    if is_thinking_variant:
        return {
            "high": {"display_name": "Think High", "strategy": "effort", "effort": "high"}
        }
    has_toggle = any(r.get("type") == "toggle" for r in ropts)
    has_none = any(
        r.get("type") == "effort" and "none" in (r.get("values") or [])
        for r in ropts
    )
    effort = None
    for r in ropts:
        if r.get("type") == "effort":
            effort = r.get("values") or []
    modes = {}
    if has_toggle or has_none:
        modes["off"] = {"display_name": "Off", "strategy": "disabled"}
    if effort:
        levels = [e for e in effort if e != "none"]
        for lv in levels:
            name, desc = EFFORT_DESC[lv]
            modes[lv] = {
                "display_name": name,
                "description": desc,
                "strategy": "effort",
                "effort": lv,
            }
        # default = the standard middle level when present
        for cand in ("medium", "high"):
            if cand in levels:
                modes["default"] = cand
                break
    return modes

patch_models = {}
for mid, ropts in sorted(TARGETS.items()):
    # verify the ropts in the live models.dev snapshot actually match
    m = dev.get(mid)
    if m is None:
        print(f"WARN {mid}: absent from models.dev, using given ropts")
    elif json.dumps(m.get("reasoning_options")) != json.dumps(ropts):
        print(f"WARN {mid}: models.dev ropts differ\n  given {json.dumps(ropts)}\n  live  {json.dumps(m.get('reasoning_options'))}")
    is_thinking = mid.endswith(":thinking")
    modes = build_modes(ropts, is_thinking_variant=is_thinking)
    patch_models[mid] = {"thinking_modes": modes}

patch = {
    "models": patch_models,
    "notes": (
        "thinking_modes for 18 models where models.dev reasoning_options is non-empty "
        "(authoritative reasoning control). Toggle+effort -> off(disabled)+ladder; effort "
        "with 'none' -> off+ladder; pure effort -> ladder. Nemotron :thinking variants keep "
        "effort-only ladders per ropts. openai-gpt-4o-mini effort low/medium/high per ropts."
    ),
}
out = os.path.join(D, "patches", "ropt_modes.json")
json.dump(patch, open(out, "w"), ensure_ascii=False, indent=2)
print(f"wrote {len(patch_models)} models -> {os.path.basename(out)}")
