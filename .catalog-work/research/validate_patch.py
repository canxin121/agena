import json, sys

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(D + "/models.merged.json"))["models"]
patch = json.load(open(D + "/patches/misc_long_tail.json"))

ALLOWED_TOP = {
    "thinking_modes", "speed_modes", "max_input_tokens", "context_window_tokens",
    "max_output_tokens", "knowledge_cutoff", "description", "pricing", "release_date",
    "open_weights", "display_name",
}
THINK_STRATS = {"disabled", "effort", "budget", "adaptive", "request_only"}
EFFORTS = {"minimal", "low", "medium", "high", "xhigh", "max"}

errors = []
models = patch["models"]
for mid, pv in models.items():
    if mid not in cat:
        errors.append(f"model {mid} NOT IN CATALOG")
        continue
    for k in pv:
        if k not in ALLOWED_TOP:
            errors.append(f"{mid}: unknown field {k}")
    tm = pv.get("thinking_modes")
    if tm is not None:
        if not isinstance(tm, dict):
            errors.append(f"{mid}: thinking_modes not dict")
        for mk, mv in tm.items():
            if mk == "default":
                continue
            if not isinstance(mv, dict):
                errors.append(f"{mid}: mode {mk} not dict")
                continue
            s = mv.get("strategy")
            if s not in THINK_STRATS:
                errors.append(f"{mid}: mode {mk} bad strategy {s}")
            if s == "effort" and mv.get("effort") not in EFFORTS:
                errors.append(f"{mid}: mode {mk} bad effort {mv.get('effort')}")
    sm = pv.get("speed_modes")
    if sm is not None:
        if not isinstance(sm, dict):
            errors.append(f"{mid}: speed_modes not dict")
        else:
            for mk, mv in sm.items():
                if not isinstance(mv, dict) or "display_name" not in mv:
                    errors.append(f"{mid}: speed mode {mk} malformed")

print("patch models:", len(models))
print("errors:", len(errors))
for e in errors[:40]:
    print("  ", e)
sys.exit(1 if errors else 0)
