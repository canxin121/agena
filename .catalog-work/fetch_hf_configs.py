#!/usr/bin/env python3
"""Batch-fetch HF config.json for the 193 models missing context caps, parse
context-length fields (max_position_embeddings / n_positions / sliding_window /
rope_scaling). Tries known HF org/ID mappings; records successes + failures.
Uses curl -L (HF redirects) via subprocess to avoid WebSearch budget.

Usage: python3 fetch_hf_configs.py
Writes research/hf_configs.json: {model_id: {ctx, source_url, raw_fields}}"""
import json, os, subprocess, time, urllib.parse

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
missing = [mid for mid, x in cat.items() if x.get("context_window_tokens") is None]

# known HF org -> id mappings (best-effort; script tries several variants)
HF_IDS = {}
def add(org, ids):
    for i in ids:
        HF_IDS[i] = org

add("ibm-granite", ["granite-20b-code-instruct-8k", "granite-20b-code-instruct-16k",
                    "granite-20b-code-instruct-32k", "granite-20b-code-instruct-128k",
                    "granite-13b-instruct-v2", "granite-13b-chat-v2",
                    "granite-7b-instruct-v2", "granite-7b-instruct-4k",
                    "granite-8b-instruct-128k", "granite-8b-instruct-4k",
                    "granite-3.1-2b-instruct", "granite-3.1-8b-instruct",
                    "granite-3.2-2b-instruct", "granite-3.2-8b-instruct",
                    "granite-3.3-2b-instruct", "granite-3.3-8b-instruct"])
add("codellama", ["CodeLlama-13b-hf", "CodeLlama-34b-hf", "CodeLlama-70b-hf",
                  "CodeLlama-7b-hf", "CodeLlama-13b-Python-hf", "CodeLlama-34b-Python-hf",
                  "CodeLlama-7b-Python-hf", "CodeLlama-13b-Instruct-hf",
                  "CodeLlama-34b-Instruct-hf", "CodeLlama-70b-Instruct-hf",
                  "CodeLlama-7b-Instruct-hf"])
add("meta-llama", ["Llama-2-7b-hf", "Llama-2-13b-hf", "Llama-2-70b-hf",
                   "Llama-2-7b-chat-hf", "Llama-2-13b-chat-hf", "Llama-2-70b-chat-hf",
                   "CodeLlama-13b-hf", "CodeLlama-34b-hf", "CodeLlama-70b-hf",
                   "Meta-Llama-Guard-2-8B"])
add("nvidia", ["Llama-3.1-Nemotron-51B-Instruct-HF", "Llama-3.1-Nemotron-8B-ultralong-1M-instruct",
               "Llama-3.1-Nemotron-Nano-4B-v1.1", "Llama-3.1-Nemotron-Ultra-253B-v1",
               "Nemotron-4-340B-Instruct", "Nemotron-Nano-3-30B-A3B-Instruct",
               "Nemotron-Parse", "Nemotron-4-340B-Base"])
add("google", ["gemma-7b", "gemma-2b", "gemma-2-9b-it", "gemma-2-9b",
               "gemma-2-27b-it", "gemma-2-27b", "gemma-3-1b-it", "gemma-3-27b-it"])
add("mistralai", ["Mixtral-8x7B-v0.1", "Mixtral-8x22B-v0.1", "Mistral-Large-Instruct-2407",
                  "Mistral-Large-2", "Mistral-7B-Instruct-v0.1", "Codestral-22B-v0.1",
                  "Mamba-Codestral-7B-v0.1", "Mistral-Nemo-Base-2407"])
add("ai21labs", ["Jamba-v0.1", "Jamba-1.5-Large", "Jamba-1.5-Mini", "Jamba-1.6-Large",
                 "Jamba-1.6-Mini", "Jamba-1.7-Large", "Jamba-1.7-Mini", "Jamba-3B-Instruct"])
add("bigcode", ["starcoder2-3b", "starcoder2-7b", "starcoder2-15b", "starcoder2-3b-instruct-v0.1"])
add("ibm", ["granite-7b-instruct-v2", "granite-13b-instruct-v2"])
add("snowflake", ["arctic-embed-l", "arctic-embed-m-long", "snowflake-arctic-embed-l",
                  "snowflake-arctic-embed-m-long", "snowflake-arctic-embed-s"])
add("microsoft", ["phi-3-vision-128k-instruct"])
add("NVIDIA", ["Llama-3.1-Nemotron-Nano-VL-8B-v1-Mcore", "Llama-3.3-Nemotron-Super-49B-v1",
               "Llama-3.3-Nemotron-Super-49B-v1.5", "Nemotron-Nano-12B-v2-VL",
               "NV-EmbedQA-E5-V5", "Llama-3.2-NV-EmbedQA-1B-v1", "Llama-3.2-NV-EmbedQA-1B-v2",
               "NVCLIP-ViT-B-16-DataComp-0.1", "Nemotron-3-Super-120B-A12B-Instruct",
               "Nemotron-3-Ultra-550B-A55B-Instruct", "Nemotron-3.5-Lightning-Instruct",
               "Nemotron-Content-Safety-Reasoning-4B"])
add("Writer", ["palmyra-med-70b-32k", "palmyra-x-004-32k", "palmyra-fin-70b-32k"])
add("togethercomputer", ["RedPajama-INCITE-7B-Instruct", "RedPajama-INCITE-Base-7B"])

# fallback: also try lowercase org + original id, and the id itself as org/id

def ctx_from_config(cfg):
    """Extract context length from a parsed HF config dict."""
    mpe = cfg.get("max_position_embeddings")
    if mpe:
        return int(mpe)
    npos = cfg.get("n_positions")
    if npos:
        return int(npos)
    # some use model_max_length
    mml = cfg.get("model_max_length")
    if mml:
        return int(mml)
    return None

def fetch(url, timeout=25):
    try:
        r = subprocess.run(["curl", "-sL", "-m", str(timeout), url],
                           capture_output=True, text=True, timeout=timeout + 10)
        return r.stdout
    except Exception:
        return None

results = {}
failures = []
for mid in missing:
    # build candidate URLs
    candidates = []
    org = HF_IDS.get(mid)
    if org:
        candidates.append(f"https://huggingface.co/{org}/{mid}/resolve/main/config.json")
    # lowercase-org guess
    lmid = mid.lower()
    candidates.append(f"https://huggingface.co/{lmid}/{lmid}/resolve/main/config.json")
    # id-as-org (e.g. mistralai/Mistral-Large-Instruct-2407)
    candidates.append(f"https://huggingface.co/{lmid}/{mid}/resolve/main/config.json")
    got = None
    for url in candidates:
        body = fetch(url)
        if body:
            try:
                cfg = json.loads(body)
            except Exception:
                cfg = None
            if cfg and isinstance(cfg, dict):
                ctx = ctx_from_config(cfg)
                if ctx:
                    got = {"ctx": ctx, "url": url,
                           "fields": {k: cfg.get(k) for k in
                                      ("max_position_embeddings","n_positions","sliding_window","rope_scaling")}}
                    break
        time.sleep(0.15)  # be gentle to HF
    if got:
        results[mid] = got
    else:
        failures.append(mid)
    if len(results) % 25 == 0 and results:
        print(f"  ... {len(results)} fetched / {len(failures)} failed")

out = os.path.join(D, "research", "hf_configs.json")
json.dump({"fetched": results, "failed": failures},
          open(out, "w"), ensure_ascii=False, indent=2)
print(f"\nFETCHED {len(results)} / {len(failures)} failed of {len(missing)}")
print("failed:", ", ".join(failures[:60]))
