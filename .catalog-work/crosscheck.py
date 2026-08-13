#!/usr/bin/env python3
"""Cross-check: which catalog models are covered by models.dev, and what fields
the models.dev entry would fill."""
import json, os, re

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"

def load(p):
    with open(os.path.join(D, p)) as f:
        return json.load(f)

cat = load("models.json")["models"]
md = load("models-dev.json")

# Flatten models.dev: key = raw model key, value = dict(model entry, provider)
# Also allow normalized matching (lowercase, strip source prefix) later.
dev_by_key = {}
for prov_key, prov in md.items():
    prov_id = prov.get("id") or prov_key
    prov_name = prov.get("name") or prov_key
    for model_key, model in (prov.get("models") or {}).items():
        mid = model.get("id") or model_key
        dev_by_key[mid] = {"provider": prov_key, "provider_id": prov_id,
                           "provider_name": prov_name, "model": model, "key": model_key}

# How many catalog ids have an exact-key match?
exact = 0
covered_fields = {"description": 0, "context": 0, "max_output": 0, "max_input": 0,
                  "pricing": 0, "knowledge": 0, "release_date": 0, "open_weights": 0}
miss = []
for cid in cat:
    e = dev_by_key.get(cid)
    if e is None:
        miss.append(cid)
        continue
    exact += 1
    m = e["model"]
    if cat[cid].get("description") is None and m.get("description"): covered_fields["description"] += 1
    lim = m.get("limit") or {}
    if cat[cid].get("context_window_tokens") is None and (lim.get("context") or lim.get("input")): covered_fields["context"] += 1
    if cat[cid].get("max_output_tokens") is None and lim.get("output"): covered_fields["max_output"] += 1
    if cat[cid].get("max_input_tokens") is None and lim.get("input"): covered_fields["max_input"] += 1
    if cat[cid].get("pricing") is None and m.get("cost"): covered_fields["pricing"] += 1
    if cat[cid].get("knowledge_cutoff") is None and m.get("knowledge"): covered_fields["knowledge"] += 1
    if cat[cid].get("release_date") is None and m.get("release_date"): covered_fields["release_date"] += 1
    if cat[cid].get("open_weights") is None and m.get("open_weights") is not None: covered_fields["open_weights"] += 1

print(f"catalog total: {len(cat)}")
print(f"exact-key match in models.dev: {exact}")
print(f"unmatched: {len(miss)}")
print("fields fillable from models.dev (exact-key only):")
for k, v in covered_fields.items():
    print(f"  {k}: {v}")
print("\nSample unmatched ids:")
for m in miss[:40]:
    print("  ", m)
