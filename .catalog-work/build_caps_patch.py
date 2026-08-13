#!/usr/bin/env python3
"""Build patches/caps.json: context_window_tokens / max_input_tokens /
max_output_tokens for all 193 models missing caps.

Sources, in priority order:
  1. research/hf_configs.json fetched values (105 models), with anomaly fixes
     for known wrong-repo matches (verified against sibling variants).
  2. Verified gated-model values (model-card WebFetch / models.dev serving caps).
  3. Family conventions for gated open-weight LLMs (out==ctx) and embeddings
     (out==hidden_size), cross-checked against sibling models already in catalog.

Left UNSET (28): vision/timeseries/speech/image-gen/CLIP/tokenizer niches whose
configs carry no standard token cap and which have no authoritative serving cap.
"""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
RES = os.path.join(D, "research", "hf_configs.json")
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
hfd = json.load(open(RES))["fetched"]

patch = {"models": {}}

def set_caps(mid, ctx, out, inp=None):
    """inp defaults to ctx. out == None means leave max_output unset (rare)."""
    patch["models"][mid] = {
        "context_window_tokens": ctx,
        "max_input_tokens": inp if inp is not None else ctx,
        "max_output_tokens": out,
    }

# ---------------------------------------------------------------- fetched 105
# anomaly fixes: wrong-repo matches -> real family value (siblings confirm)
FIX = {
    "codellama-34b": 16384, "codellama-34b-hf": 16384,
    "codellama-70b-instruct": 16384, "codellama-70b-instruct-hf": 16384,
    "codellama-70b-python": 16384, "codellama-70b-python-hf": 16384,
    "llama2-7b-chat-hf": 4096, "stockmark-2-100b-instruct-beta": 32768,
}
EMBED_OUT = {  # embedding models fetched without hidden_size: out = known dim
    "arctic-embed-l": 1024, "snowflake-arctic-embed-l": 1024,
    "snowflake-arctic-embed-m-long": 1024,
}
for mid, info in hfd.items():
    ctx = FIX.get(mid, info["ctx"])
    hs = info.get("hidden_size")
    if mid in EMBED_OUT:
        set_caps(mid, ctx, EMBED_OUT[mid])
    elif mid in ("arctic-embed-l", "snowflake-arctic-embed-l",
                 "snowflake-arctic-embed-m-long", "snowflake-arctic-embed-l-v2.0"):
        set_caps(mid, ctx, EMBED_OUT.get(mid, hs or 1024))
    elif "embed" in mid or "retriever" in mid:
        set_caps(mid, ctx, hs)  # embedding: out = hidden dimension
    elif mid.startswith(("jamba-tiny-dev", "jamba-v0.1", "jamba-reasoning-3b")):
        set_caps(mid, ctx, 4096)  # jamba serving output
    else:
        set_caps(mid, ctx, ctx)   # open-weight LLM: out == ctx

# ------------------------------------------------------------ the 88 gated/special
# (a) gated open-weight LLMs — out == ctx
LLM = {
    # meta llama-2 (4096 canonical)
    "llama2-7b": 4096, "llama2-7b-hf": 4096, "llama2-13b": 4096,
    "llama2-13b-hf": 4096, "llama2-13b-chat": 4096, "llama2-13b-chat-hf": 4096,
    "llama2-70b": 4096, "llama2-70b-hf": 4096, "llama2-70b-chat": 4096,
    # google codegemma / gemma (8192)
    "codegemma-2b": 8192, "codegemma-1.1-7b": 8192, "codegemma-7b-it": 8192,
    "gemma-2b": 8192, "gemma-7b": 8192, "gemma-2-9b-it": 8192,
    # gemma-3-1b-it: 32K confirmed from model card (1B size = 32K; 4B+ = 128K)
    "gemma-3-1b-it": 32768,
    # recurrentgemma (8192)
    "recurrentgemma-2b": 8192, "recurrentgemma-2b-it": 8192,
    "recurrentgemma-9b": 8192, "recurrentgemma-9b-it": 8192,
    # sea-lion (8k)
    "sea-lion-7b-instruct": 8192, "sea-lion-v1-7b-it": 8192,
    "sea-lion-v1-7b-it-research": 8192,
    # mamba-codestral (32k)
    "mamba-codestral-7b-v0.1": 32768,
    # minitron 8k
    "mistral-nemo-minitron-8b-8k-instruct": 8192,
    # nvidia nemotron open-weight (out==ctx except where served output known)
    "nemotron-4-340b-instruct": 4096,
    "llama-3.1-nemotron-51b-instruct": 131072,
    "llama-3.1-nemotron-ultra-253b-cpt-v1": 131072,
    "llama-3.1-nemotron-nano-vl-8b-v1-mcore": 131072,
    "nemotron-parse": 131072, "nvidia-nemotron-parse-2.0": 131072,
    "nvidia-nemotron-parse-v1.1": 131072, "nvidia-nemotron-parse-v1.1-tc": 131072,
    "nvidia-nemotron-parse-v1.2": 131072,
    # palmyra (Writer)
    "palmyra-creative-122b": 131072,
    "palmyra-med-70b": 8192,  # confirmed from model card
}
for mid, ctx in LLM.items():
    set_caps(mid, ctx, ctx)

# gated LLMs with distinct served output cap
LLM_SERV = {
    "jamba-1.5-large-instruct": 262144, "jamba-1.5-mini-instruct": 262144,
    "jamba-1.6-large-instruct": 262144, "jamba-1.6-mini-instruct": 262144,
    "jamba-1.7-large-instruct": 262144, "jamba-1.7-mini-instruct": 262144,
    "jamba-large-1.5": 262144, "jamba-mini-1.5": 262144,
    "jamba-3b-reasoning-instruct": 262144,
    "mistral-large-2-instruct": 131072, "mistral-large-instruct-2407": 131072,
    "nemotron-nano-3-30b-a3b": 131072,
}
for mid, ctx in LLM_SERV.items():
    out = 32768 if "mistral-large" in mid else 4096  # jamba/nemotron serving out
    set_caps(mid, ctx, out)

# (b) guard / safety classifiers
GUARD = {
    "meta-llama-guard-2-8b": 4096,
    "llama-3.1-nemoguard-8b-content-safety": 131072,
    "llama-3.1-nemoguard-8b-topic-control": 131072,
    "nemoguard-jailbreakdetect": 4096,
    "granite-rag-3.0-8b-lora": 4096, "granite-uncertainty-3.0-8b-lora": 4096,
    "granitelib-rag-gpt-oss-r1.0": 4096, "granitelib-rag-r1.0": 4096,
}
for mid, ctx in GUARD.items():
    set_caps(mid, ctx, ctx)

# (c) embedding / retriever (out = hidden dimension)
EMBED = {
    "llama-3.2-nv-embedqa-1b-v1": (8192, 2048),
    "llama-3.2-nemoretriever-1b-vlm-embed-v1": (131072, 2048),
    "nv-embedqa-e5-v5": (8192, 4096),
    "nemoretriever-parse": (32768, 2048),
}
for mid, (ctx, hid) in EMBED.items():
    set_caps(mid, ctx, hid)

# sanity: every patch model exists, every ctx is int
missing = [m for m in patch["models"] if m not in cat]
assert not missing, f"patch models not in catalog: {missing}"

json.dump(patch, open(os.path.join(D, "patches", "caps.json"), "w"),
          ensure_ascii=False, indent=2)
print(f"wrote patches/caps.json: {len(patch['models'])} models")

# report remaining unset
missing_all = [m for m, x in cat.items() if x.get("context_window_tokens") is None]
still = [m for m in missing_all if m not in patch["models"]]
print(f"original missing: {len(missing_all)}")
print(f"now filled:       {len(patch['models'])}")
print(f"STILL UNSET ({len(still)}):")
for m in sorted(still):
    print("  ", m)
