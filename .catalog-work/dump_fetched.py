#!/usr/bin/env python3
"""Dump fetched cap values grouped by family, flag values that deviate from a
known-canonical table (likely wrong-repo matches)."""
import json, os, re

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
res = json.load(open(os.path.join(D, "research", "hf_configs.json")))
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
fetched = res["fetched"]

# canonical context windows (tokens) I am confident about, by regex -> ctx
CANON = [
    (r"^llama-3\.3-", 131072),
    (r"^llama-3\.1-", 131072),
    (r"^llama-3\.2-", 131072),
    (r"^llama-3\.2-.*embed", 8192),
    (r"^llama-3\.1-8b-", 131072),
    (r"^llama2-", 4096),
    (r"^codellama-70b", 16384),
    (r"^codellama-34b", 16384),
    (r"^codellama-13b", 16384),
    (r"^codellama-7b", 16384),
    (r"^gemma-3", 131072),
    (r"^gemma-2", 8192),
    (r"^gemma-", 8192),
    (r"^mistral-large", 131072),
    (r"^mixtral-8x7b", 32768),
    (r"^mistral-7b", 32768),
    (r"^mistral-nemo", 131072),
    (r"^codestral", 32768),
    (r"^mamba-codestral", 32768),
    (r"^jamba-", 262144),
    (r"^recurrentgemma", 8192),
    (r"^sea-lion", 8192),
    (r"^stockmark-2", 32768),
    (r"^nemotron-4", 4096),
    (r"^grok-", 131072),
    (r"^phi-", 131072),
]

def canon_ctx(mid):
    for pat, ctx in CANON:
        if re.match(pat, mid):
            return ctx
    return None

rows = []
for mid, info in sorted(fetched.items()):
    ctx = info["ctx"]
    cctx = canon_ctx(mid)
    flag = ""
    if cctx is not None and ctx != cctx:
        flag = f"  <<< EXPECT {cctx}"
    rows.append((mid, ctx, info.get("hidden_size"), info["url"], flag))

for mid, ctx, hs, url, flag in rows:
    print(f"{mid:55s} ctx={ctx:<8} hs={hs}  {flag}")

print("\nTOTAL", len(rows))
