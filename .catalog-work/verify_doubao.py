#!/usr/bin/env python3
import json, copy, sys

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(D + "/models.merged.json"))
models = cat["models"]
patch = json.load(open(D + "/patches/domestic_doubao.json"))

ALLOWED_TOP = {
    "thinking_modes", "speed_modes", "max_input_tokens", "context_window_tokens",
    "max_output_tokens", "knowledge_cutoff", "description", "pricing", "release_date",
    "open_weights", "display_name",
}
THINK_STRATS = {"disabled", "effort", "budget", "adaptive", "request_only"}
errors = []
pmodels = patch.get("models")
if not isinstance(pmodels, dict):
    errors.append("no models dict")

for mid, pv in pmodels.items():
    if mid not in models:
        errors.append("model %s NOT IN CATALOG" % mid)
        continue
    for k, v in pv.items():
        if k not in ALLOWED_TOP:
            errors.append("model %s unknown field %s" % (mid, k))
        if k == "pricing" and not isinstance(v, dict):
            errors.append("model %s pricing not dict" % mid)
        if k == "description" and not isinstance(v, str):
            errors.append("model %s description not str" % mid)
        if k == "release_date" and not isinstance(v, str):
            errors.append("model %s release_date not str" % mid)
        if k in ("open_weights",) and not isinstance(v, bool):
            errors.append("model %s open_weights not bool" % mid)
    rd = pv.get("release_date")
    if rd and len(rd) != 10:
        errors.append("%s: release_date bad format %s" % (mid, rd))
    kc = pv.get("knowledge_cutoff")
    if kc and len(kc) not in (7, 10):
        errors.append("%s: knowledge_cutoff bad format %s" % (mid, kc))
    tm = pv.get("thinking_modes")
    if tm is not None:
        if not isinstance(tm, dict):
            errors.append("%s: thinking_modes not dict" % mid)
        else:
            for mk, mv in tm.items():
                if mk == "default":
                    continue
                if not isinstance(mv, dict):
                    errors.append("%s: thinking mode %s not dict" % (mid, mk))
                    continue
                strat = mv.get("strategy")
                if strat not in THINK_STRATS:
                    errors.append("%s: thinking mode %s bad strategy %s" % (mid, mk, strat))
                if strat == "effort" and mv.get("effort") not in ("minimal", "low", "medium", "high", "xhigh", "max"):
                    errors.append("%s: thinking mode %s bad effort %s" % (mid, mk, mv.get("effort")))
                if strat == "request_only" and "request_override" not in mv:
                    errors.append("%s: thinking mode %s request_only missing request_override" % (mid, mk))

test = copy.deepcopy(cat)
for mid, pv in pmodels.items():
    for k, v in pv.items():
        test["models"][mid][k] = v

for mid, pv in pmodels.items():
    pr = test["models"][mid].get("pricing")
    if pr is not None and not isinstance(pr, dict):
        errors.append("%s: resulting pricing not dict" % mid)

if errors:
    print("ERRORS:")
    for e in errors:
        print("  ", e)
    sys.exit(1)
print("VALIDATION PASSED")
print("patch covers %d models" % len(pmodels))
for mid, pv in pmodels.items():
    print("  %s: %s" % (mid, sorted(pv.keys())))
