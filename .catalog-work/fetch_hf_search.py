#!/usr/bin/env python3
"""Robust HF context-window fetcher: search HF API for each missing-cap model's
real repo id, fetch config.json, extract context (max_position_embeddings /
n_positions / sliding_window / model_max_length) + hidden_size (for embeddings).

Writes research/hf_configs.json (merges with prior results)."""
import json, os, subprocess, time, sys

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
RES = os.path.join(D, "research", "hf_configs.json")
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
missing = [mid for mid, x in cat.items() if x.get("context_window_tokens") is None]

# resume: load prior
prev = json.load(open(RES)) if os.path.exists(RES) else {"fetched": {}, "failed": missing[:]}
fetched = prev.get("fetched", {})
failed = [m for m in prev.get("failed", []) if m not in fetched]

SKIP_SUFFIX = ("-gguf", "-gptq", "-awq", "-fp8", "-4bit", "-8bit", "-mlx",
               "-pytorch", "-safetensors", "-bnb", "-quantized", "-test", "-demo",
               "-1p", "-2p", "-base-hf", "-chat-hf", "-it-hf", "-instruct-hf")

def run(cmd, timeout=30):
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout + 10)
        return r.stdout
    except Exception:
        return None

def search_repo(mid):
    """Return best HF repo id for a catalog model id."""
    q = mid
    # normalize: strip suffixes that aren't HF names
    body = run(["curl", "-sL", "-m", "25",
                "https://huggingface.co/api/models?search=" + q])
    if not body:
        return None
    try:
        results = json.loads(body)
    except Exception:
        return None
    if not results:
        return None
    # exact id match first
    for r in results:
        if r.get("id", "").lower() == q.lower():
            return r["id"]
    # else first "real" repo (skip GGUF/quantized/variants)
    for r in results:
        rid = r.get("id", "")
        low = rid.lower()
        if any(low.endswith(s) for s in SKIP_SUFFIX):
            continue
        return rid
    return None

def ctx_from_config(cfg):
    mpe = cfg.get("max_position_embeddings")
    if mpe:
        return int(mpe)
    npos = cfg.get("n_positions")
    if npos:
        return int(npos)
    mml = cfg.get("model_max_length")
    if mml:
        return int(mml)
    sw = cfg.get("sliding_window")
    if isinstance(sw, int) and sw:
        return sw
    return None

def fetch_config(org, repo):
    url = f"https://huggingface.co/{org}/{repo}/resolve/main/config.json"
    body = run(["curl", "-sL", "-m", "30", url])
    if not body:
        return None, url
    try:
        return json.loads(body), url
    except Exception:
        return None, url

new_fetched = {}
still_failed = []
for i, mid in enumerate(missing):
    if mid in fetched:
        continue
    repo = search_repo(mid)
    if not repo:
        still_failed.append(mid)
        continue
    cfg, url = fetch_config(*repo.split("/", 1))
    if not cfg or not isinstance(cfg, dict):
        still_failed.append(mid)
        continue
    ctx = ctx_from_config(cfg)
    if not ctx:
        still_failed.append(mid)
        continue
    new_fetched[mid] = {
        "ctx": ctx,
        "hidden_size": cfg.get("hidden_size") or cfg.get("d_model"),
        "url": url,
        "fields": {k: cfg.get(k) for k in
                   ("max_position_embeddings","n_positions","sliding_window",
                    "model_max_length","rope_scaling","hidden_size","d_model")},
    }
    if (i + 1) % 20 == 0:
        print(f"  ... {len(new_fetched) + len(fetched)} fetched / {len(still_failed)} failed")

all_fetched = {**fetched, **new_fetched}
json.dump({"fetched": all_fetched, "failed": sorted(still_failed)},
          open(RES, "w"), ensure_ascii=False, indent=2)
print(f"\nTOTAL fetched {len(all_fetched)} / {len(still_failed)} failed of {len(missing)}")
print("failed:", ", ".join(still_failed[:40]))
