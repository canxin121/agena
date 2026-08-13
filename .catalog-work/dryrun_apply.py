#!/usr/bin/env python3
"""Dry-run apply_patches.py logic on a copy of models.merged.json to validate
patch order, null handling, and no null values surviving. Does not mutate the
real merged file."""
import json, glob, os, shutil

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
CAT = os.path.join(D, "models.merged.json")
TMP = "/tmp/catalog-dryrun.json"
shutil.copy(CAT, TMP)

ALLOWED_TOP = {
    "thinking_modes", "speed_modes", "max_input_tokens", "context_window_tokens",
    "max_output_tokens", "knowledge_cutoff", "description", "pricing", "release_date",
    "open_weights", "display_name",
}
THINK_STRATS = {"disabled", "effort", "budget", "adaptive", "request_only"}

cat = json.load(open(TMP))
models = cat["models"]
errors = []
patched = 0

for pf in sorted(glob.glob(os.path.join(D, "patches", "*.json"))):
    try:
        patch = json.load(open(pf))
    except Exception as e:
        errors.append(f"{os.path.basename(pf)}: unparseable ({e})")
        continue
    pmodels = patch.get("models")
    if not isinstance(pmodels, dict):
        errors.append(f"{os.path.basename(pf)}: no models dict")
        continue
    for mid, pv in pmodels.items():
        if not isinstance(pv, dict):
            errors.append(f"{os.path.basename(pf)}: model {mid} not object")
            continue
        if mid not in models:
            errors.append(f"{os.path.basename(pf)}: model {mid} NOT IN CATALOG")
            continue
        cur = models[mid]
        for k, v in pv.items():
            if k not in ALLOWED_TOP:
                errors.append(f"{os.path.basename(pf)}: model {mid} unknown field {k}")
                continue
            if v is None:
                cur.pop(k, None)
                continue
            cur[k] = v
        patched += 1

# validate thinking/speed shapes for touched models (same as apply_patches.py)
for mid, cur in models.items():
    tm = cur.get("thinking_modes")
    if tm is not None:
        if not isinstance(tm, dict):
            errors.append(f"{mid}: thinking_modes not dict")
        else:
            for mk, mv in tm.items():
                if mk == "default":
                    continue
                if not isinstance(mv, dict):
                    errors.append(f"{mid}: thinking mode {mk} not dict")
                    continue
                strat = mv.get("strategy")
                if strat not in THINK_STRATS:
                    errors.append(f"{mid}: thinking mode {mk} bad strategy {strat}")
                if strat == "effort" and mv.get("effort") not in ("minimal","low","medium","high","xhigh","max"):
                    errors.append(f"{mid}: thinking mode {mk} bad effort {mv.get('effort')}")
    sm = cur.get("speed_modes")
    if sm is not None:
        if not isinstance(sm, dict):
            errors.append(f"{mid}: speed_modes not dict")
        else:
            for mk, mv in sm.items():
                if not isinstance(mv, dict) or "display_name" not in mv:
                    errors.append(f"{mid}: speed mode {mk} malformed")

# A literal null VALUE would fail Rust deserialization (these fields aren't
# Option). Check for explicit keys holding null, not mere absence.
nulls = [mid for mid, m in models.items()
         if mid in m and (m.get("thinking_modes") is None or m.get("speed_modes") is None)]
print(f"patched {patched} model-field-sets")
print(f"null thinking/speed survived: {len(nulls)}", nulls[:10])
if errors:
    print(f"\nERRORS ({len(errors)}):")
    for e in errors[:40]:
        print("  ", e)
else:
    print("dry-run: no errors")
tm = sum(1 for m in models.values() if m.get("thinking_modes"))
sm = sum(1 for m in models.values() if m.get("speed_modes"))
print(f"thinking_modes present: {tm} | speed_modes present: {sm}")
os.remove(TMP)
