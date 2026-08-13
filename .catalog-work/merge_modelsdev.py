#!/usr/bin/env python3
"""Merge authoritative base fields from models.dev into the catalog for any
model whose key matches exactly. Fill only missing fields; never overwrite
existing values. Capabilities are merged (union of supported/unsupported)."""
import json, os, sys

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"

def load(p):
    with open(os.path.join(D, p)) as f:
        return json.load(f)

cat = load("models.json")
md = load("models-dev.json")

# index models.dev by exact model id
dev = {}
for prov in md.values():
    for mid, m in (prov.get("models") or {}).items():
        real = m.get("id") or mid
        dev.setdefault(real, m)
        dev.setdefault(mid, m)

def usd(v):
    if v is None:
        return None
    if isinstance(v, (int, float)):
        return ("%.2f" % v).rstrip("0").rstrip(".") if v == int(v) else ("%g" % v)
    s = str(v).strip()
    return s if s and s not in ("0", "0.0") else None

def pricing_from_cost(cost):
    if not cost:
        return None
    p = {}
    def num(k):
        v = cost.get(k)
        return usd(v)
    p["input_usd_per_million_tokens"] = num("input")
    p["output_usd_per_million_tokens"] = num("output")
    p["cache_read_usd_per_million_tokens"] = num("cache_read")
    p["cache_write_usd_per_million_tokens"] = num("cache_write")
    # tiers
    tiers = []
    for t in cost.get("tiers") or []:
        d = {}
        if t.get("type"): d["tier_type"] = t["type"]
        if t.get("size"): d["size_tokens"] = t["size"]
        d["input_usd_per_million_tokens"] = num2(t.get("input"))
        d["output_usd_per_million_tokens"] = num2(t.get("output"))
        if any(d.values()):
            tiers.append(d)
    if tiers:
        p["tiers"] = tiers
    if any(v for v in p.values() if not isinstance(v, list)):
        return p
    return None

def num2(v):
    return usd(v)

def cap_from_model(m):
    """Build ModelCapabilityPatch from models.dev signals."""
    inp = m.get("modalities") or {}
    inputs = []
    for x in inp.get("input") or []:
        x = x.lower()
        if x in ("text", "image", "audio", "video", "pdf", "file"):
            inputs.append(x)
    patch = {}
    if inputs:
        patch["input"] = {"supported": inputs}
    features_sup = []
    features_unsup = []
    if m.get("reasoning") is True: features_sup.append("reasoning")
    if m.get("tool_call") is True: features_sup.append("tool_calling")
    if m.get("structured_output") is True: features_sup.append("structured_output")
    if m.get("temperature") is True: features_sup.append("temperature")
    if m.get("reasoning") is False: features_unsup.append("reasoning")
    if m.get("tool_call") is False: features_unsup.append("tool_calling")
    if m.get("structured_output") is False: features_unsup.append("structured_output")
    if m.get("temperature") is False: features_unsup.append("temperature")
    if features_sup or features_unsup:
        feats = {}
        if features_sup: feats["supported"] = features_sup
        if features_unsup: feats["unsupported"] = features_unsup
        patch["features"] = feats
    return patch if patch else None

# merge capabilities union
def merge_cap(existing, patch):
    if not patch:
        return existing
    def union(ex, key, patchval):
        ex_set = set((ex or {}).get(key) or [])
        ex_set.update(patchval or [])
        return sorted(ex_set)
    if existing is None:
        return patch
    result = dict(existing)
    for key in ("input", "features"):
        if key in patch:
            old = result.get(key)
            if isinstance(old, list):
                # legacy array form
                vals = union(None, key, patch[key].get("supported"))
                result[key] = vals
            elif isinstance(old, dict):
                nd = dict(old)
                if key in patch and isinstance(patch[key], dict):
                    if "supported" in patch[key]:
                        nd["supported"] = union(old, "supported", patch[key]["supported"])
                    if "unsupported" in patch[key]:
                        nd["unsupported"] = union(old, "unsupported", patch[key]["unsupported"])
                result[key] = nd
            else:
                result[key] = patch[key]
    return result

stats = {k: 0 for k in ("description", "knowledge_cutoff", "context", "max_output",
                        "pricing", "capabilities", "release_date", "open_weights", "max_input")}
matched = 0
for cid, cdef in cat["models"].items():
    m = dev.get(cid)
    if m is None:
        # try lowercase
        m = dev.get(cid.lower())
    if m is None:
        continue
    matched += 1
    if cdef.get("description") is None and m.get("description"):
        cdef["description"] = m["description"]; stats["description"] += 1
    if cdef.get("knowledge_cutoff") is None and m.get("knowledge"):
        cdef["knowledge_cutoff"] = m["knowledge"]; stats["knowledge_cutoff"] += 1
    if cdef.get("release_date") is None and m.get("release_date"):
        cdef["release_date"] = m["release_date"]; stats["release_date"] += 1
    if cdef.get("open_weights") is None and m.get("open_weights") is not None:
        cdef["open_weights"] = m["open_weights"]; stats["open_weights"] += 1
    lim = m.get("limit") or {}
    if cdef.get("context_window_tokens") is None and (lim.get("context") or lim.get("input")):
        cdef["context_window_tokens"] = lim.get("context") or lim.get("input"); stats["context"] += 1
    if cdef.get("max_output_tokens") is None and lim.get("output"):
        cdef["max_output_tokens"] = lim["output"]; stats["max_output"] += 1
    if cdef.get("max_input_tokens") is None and lim.get("input"):
        cdef["max_input_tokens"] = lim["input"]; stats["max_input"] += 1
    if cdef.get("pricing") is None:
        p = pricing_from_cost(m.get("cost"))
        if p:
            cdef["pricing"] = p; stats["pricing"] += 1
    if cdef.get("capabilities") is None:
        cap = cap_from_model(m)
        if cap:
            cdef["capabilities"] = cap; stats["capabilities"] += 1
    else:
        cap = cap_from_model(m)
        if cap:
            merged = merge_cap(cdef["capabilities"], cap)
            if merged != cdef["capabilities"]:
                cdef["capabilities"] = merged

print(f"matched exact ids: {matched}/{len(cat['models'])}")
print("fields filled from models.dev:")
for k, v in stats.items():
    print(f"  {k}: {v}")

out = os.path.join(D, "models.merged.json")
with open(out, "w") as f:
    json.dump(cat, f, ensure_ascii=False, indent=2)
print("wrote", out)
