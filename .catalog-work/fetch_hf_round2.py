#!/usr/bin/env python3
"""Round 2: fetch configs for the 96 failed + re-verify suspicious values, using
exact known HF repo mappings (org/id) instead of search API. Records ctx +
hidden_size + whether rope_scaling extends context."""
import json, os, subprocess

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
RES = os.path.join(D, "research", "hf_configs.json")

# exact HF repo -> catalog ids they map to (org, repo)
KNOWN = {
    # codegemma
    "codegemma-1.1-7b": ("google", "codegemma-1.1-7b"),
    "codegemma-2b": ("google", "codegemma-2b"),
    "codegemma-7b-it": ("google", "codegemma-7b-it"),
    # codellama (base, no suffix)
    "codellama-13b": ("codellama", "CodeLlama-13b-hf"),
    "codellama-13b-instruct-hf": ("codellama", "CodeLlama-13b-Instruct-hf"),
    "codellama-34b-instruct-hf": ("codellama", "CodeLlama-34b-Instruct-hf"),
    "codellama-70b-instruct": ("codellama", "CodeLlama-70b-Instruct-hf"),
    "codellama-7b-instruct": ("codellama", "CodeLlama-7b-Instruct-hf"),
    "codellama-7b-instruct-hf": ("codellama", "CodeLlama-7b-Instruct-hf"),
    # mistral
    "codestral-22b-instruct-v0.1": ("mistralai", "Codestral-22B-v0.1"),
    "mistral-large-2-instruct": ("mistralai", "Mistral-Large-Instruct-2407"),
    "mistral-large-instruct-2407": ("mistralai", "Mistral-Large-Instruct-2407"),
    "mistral-nemo-minitron-8b-8k-instruct": ("nvidia", "Mistral-NeMo-Minitron-8B-8K-Instruct"),
    "mamba-codestral-7b-v0.1": ("mistralai", "Mamba-Codestral-7B-v0.1"),
    # gemma
    "gemma-2-9b-it": ("google", "gemma-2-9b-it"),
    "gemma-2b": ("google", "gemma-2b"),
    "gemma-3-1b-it": ("google", "gemma-3-1b-it"),
    "gemma-7b": ("google", "gemma-7b"),
    # llama2
    "llama2-13b": ("meta-llama", "Llama-2-13b-hf"),
    "llama2-13b-chat": ("meta-llama", "Llama-2-13b-chat-hf"),
    "llama2-13b-chat-hf": ("meta-llama", "Llama-2-13b-chat-hf"),
    "llama2-13b-hf": ("meta-llama", "Llama-2-13b-hf"),
    "llama2-70b": ("meta-llama", "Llama-2-70b-hf"),
    "llama2-70b-chat": ("meta-llama", "Llama-2-70b-chat-hf"),
    "llama2-70b-hf": ("meta-llama", "Llama-2-70b-hf"),
    "llama2-7b": ("meta-llama", "Llama-2-7b-hf"),
    "llama2-7b-hf": ("meta-llama", "Llama-2-7b-hf"),
    "meta-llama-guard-2-8b": ("meta-llama", "Meta-Llama-Guard-2-8B"),
    # nvidia / nemotron
    "llama-3.1-nemoguard-8b-content-safety": ("nvidia", "Llama-3.1-Nemoguard-8B-Content-Safety"),
    "llama-3.1-nemoguard-8b-topic-control": ("nvidia", "Llama-3.1-Nemoguard-8B-Topic-Control"),
    "llama-3.1-nemotron-51b-instruct": ("nvidia", "Llama-3.1-Nemotron-51B-Instruct-HF"),
    "llama-3.1-nemotron-nano-vl-8b-v1-mcore": ("nvidia", "Llama-3.1-Nemotron-Nano-VL-8B-v1-Mcore"),
    "llama-3.1-nemotron-ultra-253b-cpt-v1": ("nvidia", "Llama-3.1-Nemotron-Ultra-253B-CPT-v1"),
    "llama-3.2-nemoretriever-1b-vlm-embed-v1": ("nvidia", "Llama-3.2-Nemoretriever-1B-VLM-Embed-v1"),
    "llama-3.2-nv-embedqa-1b-v1": ("nvidia", "Llama-3.2-NV-EmbedQA-1B-v1"),
    "llama-nemotron-embed-1b-v2": ("nvidia", "Llama-Nemotron-Embed-1B-v2"),
    "nemotron-4-340b-instruct": ("nvidia", "Nemotron-4-340B-Instruct"),
    "nemotron-nano-3-30b-a3b": ("nvidia", "Nemotron-Nano-3-30B-A3B-Instruct"),
    "nv-embedqa-e5-v5": ("nvidia", "NV-EmbedQA-E5-V5"),
    "nvclip": ("nvidia", "NVCLIP-ViT-B-16-DataComp-0.1"),
    # jamba (ai21)
    "jamba-1.5-large-instruct": ("ai21labs", "Jamba-1.5-Large"),
    "jamba-1.5-mini-instruct": ("ai21labs", "Jamba-1.5-Mini"),
    "jamba-1.6-large-instruct": ("ai21labs", "Jamba-1.6-Large"),
    "jamba-1.6-mini-instruct": ("ai21labs", "Jamba-1.6-Mini"),
    "jamba-1.7-large-instruct": ("ai21labs", "Jamba-1.7-Large"),
    "jamba-1.7-mini-instruct": ("ai21labs", "Jamba-1.7-Mini"),
    "jamba-3b-reasoning-instruct": ("ai21labs", "Jamba-3B-Instruct"),
    "jamba-large-1.5": ("ai21labs", "Jamba-1.5-Large"),
    "jamba-mini-1.5": ("ai21labs", "Jamba-1.5-Mini"),
    # recurrentgemma
    "recurrentgemma-2b": ("google", "recurrentgemma-2b"),
    "recurrentgemma-2b-it": ("google", "recurrentgemma-2b-it"),
    "recurrentgemma-9b": ("google", "recurrentgemma-9b"),
    "recurrentgemma-9b-it": ("google", "recurrentgemma-9b-it"),
    # sea-lion (aisingapore)
    "sea-lion-7b-instruct": ("aisingapore", "sea-lion-7b-instruct"),
    "sea-lion-v1-7b-it": ("aisingapore", "sea-lion-7b-instruct"),
    "sea-lion-v1-7b-it-research": ("aisingapore", "sea-lion-7b-instruct"),
}

def run(cmd, timeout=30):
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout + 10)
        return r.stdout
    except Exception:
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

data = json.load(open(RES))
fetched = data.get("fetched", {})
for mid, (org, repo) in KNOWN.items():
    if mid in fetched:
        continue
    url = f"https://huggingface.co/{org}/{repo}/resolve/main/config.json"
    body = run(["curl", "-sL", "-m", "30", url])
    if not body:
        continue
    try:
        cfg = json.loads(body)
    except Exception:
        continue
    ctx = ctx_from_config(cfg)
    if not ctx:
        continue
    fetched[mid] = {
        "ctx": ctx,
        "hidden_size": cfg.get("hidden_size") or cfg.get("d_model"),
        "url": url,
        "fields": {k: cfg.get(k) for k in
                   ("max_position_embeddings","n_positions","sliding_window",
                    "model_max_length","rope_scaling","hidden_size","d_model")},
    }
    print(f"fetched {mid}: ctx={ctx} hidden={fetched[mid]['hidden_size']}")

# recompute failed
missing_all = [mid for mid, x in
               json.load(open(os.path.join(D, "models.merged.json")))["models"].items()
               if x.get("context_window_tokens") is None]
failed = [mid for mid in missing_all if mid not in fetched]
json.dump({"fetched": fetched, "failed": sorted(failed)},
          open(RES, "w"), ensure_ascii=False, indent=2)
print(f"\nTOTAL fetched {len(fetched)} / {len(failed)} failed")
print("failed:", ", ".join(failed[:50]))
