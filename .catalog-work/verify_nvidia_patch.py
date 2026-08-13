#!/usr/bin/env python3
"""Cross-check the nvidia patch's thinking modes against models.dev
reasoning_options. Same rigor as other bundles."""
import json, os
D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"

patch = json.load(open(os.path.join(D, "patches/nvidia.json")))["models"]
md = json.load(open(os.path.join(D, "research/models.dev.json")))
dev = {}
for prov in md.values():
    for mid, m in (prov.get("models") or {}).items():
        real = m.get("id") or mid
        dev.setdefault(real, m)
        dev.setdefault(mid, m)

def patch_has_toggle(tm):
    for k, v in (tm or {}).items():
        if k == "default": continue
        if isinstance(v, dict) and v.get("strategy") == "request_only":
            return True
    return False

def patch_has_effort(tm):
    for k, v in (tm or {}).items():
        if k == "default": continue
        if isinstance(v, dict) and v.get("strategy") == "effort":
            return True
    return False

def ropts_kind(ro):
    if not ro: return None
    for r in ro:
        t = r.get("type")
        if t == "effort": return "effort"
        if t == "toggle": return "toggle"
        if t == "token": return "budget"
    return "other"

mismatch, unsupported, absent, ok = [], [], [], []

for mid, pv in sorted(patch.items()):
    tm = pv.get("thinking_modes")
    m = dev.get(mid)
    if m is None:
        absent.append(mid); continue
    ro = m.get("reasoning_options")
    kind = ropts_kind(ro)
    has_t = patch_has_toggle(tm)
    has_e = patch_has_effort(tm)
    if kind == "toggle" and has_e and not has_t:
        mismatch.append((mid, "patch=effort but ropts=toggle", ro))
    elif kind == "effort" and has_t and not has_e:
        mismatch.append((mid, "patch=toggle but ropts=effort", ro))
    elif kind in ("effort", "toggle") and (has_t or has_e):
        ok.append((mid, kind, ro))
    elif kind is None:
        unsupported.append((mid, "ropts empty", ro))
    else:
        unsupported.append((mid, f"ropts kind={kind}", ro))

print("=== MISMATCH (patch contradicts models.dev) ===")
for mid, why, ro in mismatch:
    print(f"  {mid}: {why}  ropts={json.dumps(ro)}")
print(f"\n=== ropts empty (template unsupported) — {len(unsupported)} ===")
for mid, why, ro in unsupported:
    print(f"  {mid}")
print(f"\n=== ABSENT from models.dev — {len(absent)} ===")
print("  " + ", ".join(absent))
print(f"\n=== OK (matches ropts) — {len(ok)} ===")
for mid, kind, ro in ok:
    print(f"  {mid}: {kind}")
