#!/usr/bin/env python3
"""Show existing cap conventions for embedding / guard / jamba / codellama /
arctic / snowflake / granite entries already in the catalog."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

WANT = ["bge-m3", "gemini-embedding", "text-embedding", "granite-embedding",
        "granite-guardian", "jamba", "codellama", "arctic-embed", "snowflake-arctic",
        "llama2", "nv-embed", "nemoretriever", "nemotron", "palmyra", "zamba",
        "sea-lion", "recurrentgemma", "codegemma", "gemma-3", "kosmos", "vila",
        "deplot", "fuyu", "phi-3-vision", "starcoder2"]
seen = set()
for mid, e in cat.items():
    for w in WANT:
        if w in mid.lower():
            key = mid.split("-")[0] if not mid.split("-")[0] in ("granite", "snowflake", "codellama", "gemma", "llama2", "arctic", "nv", "sea", "phi", "star", "palmyra", "zamba", "jamba", "kosmos", "recurrentgemma", "codegemma", "bge", "text", "deplot", "fuyu", "vila", "nemotron", "nemoretriever") else mid.split("-")[0] + "-" + (mid.split("-")[1] if len(mid.split("-"))>1 else "")
            if mid in seen:
                break
            seen.add(mid)
            print(f"{mid:55s} ctx={e.get('context_window_tokens')} in={e.get('max_input_tokens')} out={e.get('max_output_tokens')} open={e.get('open_weights')}")
            break
