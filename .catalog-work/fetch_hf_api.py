#!/usr/bin/env python3
"""Try HF API ?expand[]=config for gated/uncertain models. Public metadata API
sometimes exposes config even for gated repos."""
import json, os, subprocess

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"

TARGETS = [
    # (catalog-id, HF repo)
    ("gemma-3-1b-it", "google/gemma-3-1b-it"),
    ("gemma-2b", "google/gemma-2b"),
    ("gemma-7b", "google/gemma-7b"),
    ("gemma-2-9b-it", "google/gemma-2-9b-it"),
    ("recurrentgemma-2b", "google/recurrentgemma-2b"),
    ("recurrentgemma-9b-it", "google/recurrentgemma-9b-it"),
    ("jamba-1.5-large-instruct", "ai21labs/Jamba-1.5-Large"),
    ("jamba-1.6-mini-instruct", "ai21labs/Jamba-1.6-Mini"),
    ("jamba-1.7-large-instruct", "ai21labs/Jamba-1.7-Large"),
    ("jamba-3b-reasoning-instruct", "ai21labs/Jamba-3B-Instruct"),
    ("mistral-large-2-instruct", "mistralai/Mistral-Large-Instruct-2407"),
    ("mistral-large-instruct-2407", "mistralai/Mistral-Large-Instruct-2407"),
    ("mamba-codestral-7b-v0.1", "mistralai/Mamba-Codestral-7B-v0.1"),
    ("mistral-nemo-minitron-8b-8k-instruct", "nvidia/Mistral-NeMo-Minitron-8B-8K-Instruct"),
    ("llama2-7b", "meta-llama/Llama-2-7b-hf"),
    ("llama2-70b-chat", "meta-llama/Llama-2-70b-chat-hf"),
    ("meta-llama-guard-2-8b", "meta-llama/Meta-Llama-Guard-2-8B"),
    ("llama-3.1-nemotron-51b-instruct", "nvidia/Llama-3.1-Nemotron-51B-Instruct-HF"),
    ("llama-3.1-nemotron-ultra-253b-cpt-v1", "nvidia/Llama-3.1-Nemotron-Ultra-253B-CPT-v1"),
    ("nemotron-nano-3-30b-a3b", "nvidia/Nemotron-Nano-3-30B-A3B-Instruct"),
    ("llama-3.1-nemoguard-8b-content-safety", "nvidia/Llama-3.1-Nemoguard-8B-Content-Safety"),
    ("llama-3.2-nemoretriever-1b-vlm-embed-v1", "nvidia/Llama-3.2-Nemoretriever-1B-VLM-Embed-v1"),
    ("llama-nemotron-embed-1b-v2", "nvidia/Llama-Nemotron-Embed-1B-v2"),
    ("nv-embedqa-e5-v5", "nvidia/NV-EmbedQA-E5-V5"),
    ("sea-lion-7b-instruct", "aisingapore/sea-lion-7b-instruct"),
    ("zamba2-7b-instruct", "Zyphra/Zamba2-7B-Instruct"),
    ("codegemma-2b", "google/codegemma-2b"),
    ("codegemma-1.1-7b", "google/codegemma-1.1-7b"),
    ("palmyra-med-70b", "Writer/Palmyra-Med-70B"),
]

def run(cmd, timeout=25):
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout + 10)
        return r.stdout
    except Exception:
        return None

out = {}
for mid, repo in TARGETS:
    body = run(["curl", "-sL", "-m", "25",
                f"https://huggingface.co/api/models/{repo}?expand[]=config&expand[]=cardData"])
    if not body:
        print(f"{mid}: NO BODY")
        continue
    try:
        j = json.loads(body)
    except Exception:
        print(f"{mid}: bad json")
        continue
    cfg = j.get("config") or {}
    mpe = cfg.get("max_position_embeddings") or cfg.get("model_max_length") or cfg.get("n_positions")
    sw = cfg.get("sliding_window")
    hs = cfg.get("hidden_size") or cfg.get("d_model")
    print(f"{mid}: gated={j.get('gated')} ctx={mpe} sw={sw} hs={hs} url={repo}")
    out[mid] = {"repo": repo, "gated": j.get("gated"), "ctx": mpe, "sliding_window": sw, "hidden_size": hs,
                "config_keys": sorted(cfg.keys())[:40] if cfg else []}

json.dump(out, open(os.path.join(D, "research", "hf_api_configs.json"), "w"),
          ensure_ascii=False, indent=2)
print("\nwritten research/hf_api_configs.json")
