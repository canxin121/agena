#!/usr/bin/env python3
"""Full-document validation mirroring serde deny_unknown_fields + enum checks
across every model in the merged catalog."""
import json, os, sys
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))
models = cat["models"]

ALLOWED = {
    "lifecycle", "context_window_tokens", "max_input_tokens", "max_output_tokens",
    "description", "knowledge_cutoff", "release_date", "last_updated", "open_weights",
    "supports_parallel_tool_calls", "supports_verbosity", "default_verbosity",
    "default_temperature", "default_top_p", "default_top_k",
    "assistant_reasoning_interleaved", "assistant_reasoning_field",
    "output_modalities", "pricing", "display_name", "origin",
    "thinking_modes", "speed_modes", "input", "features",
}
LIFECYCLE = {"active", "preview", "beta", "alpha", "experimental", "deprecated"}
STRATS = {"disabled", "effort", "budget", "adaptive", "request_only"}
EFFORTS = {"minimal", "low", "medium", "high", "xhigh", "max"}
INPUT_MODS = {"text", "image", "document", "audio", "video", "file"}
FEATURES = {"tool_calling", "streaming", "reasoning", "structured_output", "temperature"}
PRICING_KEYS = {"input_usd_per_million_tokens", "output_usd_per_million_tokens",
                "cache_read_usd_per_million_tokens", "cache_write_usd_per_million_tokens",
                "tiers"}
TIER_KEYS = {"tier_type", "size_tokens", "input_usd_per_million_tokens",
             "output_usd_per_million_tokens", "cache_read_usd_per_million_tokens",
             "cache_write_usd_per_million_tokens"}

errors = []
n = 0

def check(cond, msg):
    if not cond:
        errors.append(msg)

for mid, c in models.items():
    n += 1
    for k in c:
        if k not in ALLOWED:
            errors.append(f"{mid}: unknown key {k!r}")
            continue
    if c.get("lifecycle") is not None and c["lifecycle"] not in LIFECYCLE:
        errors.append(f"{mid}: bad lifecycle {c['lifecycle']}")
    # numeric limits: u32; 0 is a pre-existing convention for "unknown/unlimited"
    for f in ("context_window_tokens", "max_input_tokens", "max_output_tokens"):
        v = c.get(f)
        if v is not None and (not isinstance(v, int) or v < 0):
            errors.append(f"{mid}: bad {f}={v!r}")
    # capabilities: input/features forms
    for f in ("input", "features"):
        v = c.get(f)
        if v is not None:
            if isinstance(v, list):
                allowed = INPUT_MODS if f == "input" else FEATURES
                for x in v:
                    if x not in allowed:
                        errors.append(f"{mid}: {f} list bad {x!r}")
            elif isinstance(v, dict):
                for sub in ("supported", "unsupported"):
                    lst = v.get(sub)
                    if lst is not None:
                        allowed = INPUT_MODS if f == "input" else FEATURES
                        for x in lst:
                            if x not in allowed:
                                errors.append(f"{mid}: {f}.{sub} bad {x!r}")
            else:
                errors.append(f"{mid}: {f} bad type {type(v)}")
    # thinking modes
    tm = c.get("thinking_modes") or {}
    for mk, mv in tm.items():
        if mk == "default":
            if mv not in tm:
                errors.append(f"{mid}: default thinking {mv!r} not a mode")
            continue
        if not isinstance(mv, dict):
            errors.append(f"{mid}: thinking mode {mk} not dict")
            continue
        if mv.get("strategy") not in STRATS:
            errors.append(f"{mid}: thinking {mk} bad strategy {mv.get('strategy')}")
        if mv.get("effort") is not None and mv["effort"] not in EFFORTS:
            errors.append(f"{mid}: thinking {mk} bad effort {mv['effort']}")
        if mv.get("budget_tokens") is not None and not isinstance(mv["budget_tokens"], int):
            errors.append(f"{mid}: thinking {mk} bad budget_tokens")
    # speed modes
    sm = c.get("speed_modes") or {}
    for mk, mv in sm.items():
        if mk == "default":
            continue
        if not isinstance(mv, dict) or not mv.get("display_name"):
            errors.append(f"{mid}: speed mode {mk} malformed")
    # pricing
    pr = c.get("pricing")
    if pr is not None:
        if not isinstance(pr, dict):
            errors.append(f"{mid}: pricing not dict")
        else:
            for k in pr:
                if k not in PRICING_KEYS:
                    errors.append(f"{mid}: pricing unknown key {k!r}")
            for t in pr.get("tiers") or []:
                if not isinstance(t, dict):
                    errors.append(f"{mid}: pricing tier not dict")
                    continue
                for k in t:
                    if k not in TIER_KEYS:
                        errors.append(f"{mid}: pricing tier unknown key {k!r}")

print(f"validated {n} models")
if errors:
    print(f"\nERRORS ({len(errors)}):")
    for e in errors[:80]:
        print("  ", e)
    sys.exit(1)
print("ALL PASS: no unknown keys, valid enums, valid pricing/limits")
