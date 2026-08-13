#!/usr/bin/env python3
"""Round 3: exact-repo fetch for public (non-gated) stragglers + HF search for
the remaining nvidia specials. Writes into research/hf_configs.json."""
import json, os, subprocess

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
RES = os.path.join(D, "research", "hf_configs.json")
data = json.load(open(RES))
fetched = data["fetched"]

EXACT = {
    "kosmos-2": ("microsoft", "kosmos-2"),
    "kosmos-2-patch14-224": ("microsoft", "kosmos-2-patch14-224"),
    "kosmos-2.5": ("microsoft", "Kosmos-2.5"),
    "kosmos-2.5-chat": ("microsoft", "Kosmos-2.5"),
    "deplot": ("google", "deplot"),
    "vila": ("Efficient-Large-Model", "VILA1.5-3b"),
    "starcoder2-tokenizer": ("bigcode", "starcoder2-tokenizer"),
    "nv-embedqa-e5-v5": ("nvidia", "NV-EmbedQA-E5-V5"),
    "nvclip": ("nvidia", "NVCLIP-ViT-B-16-DataComp-0.1"),
    "granite-docling-258m": ("ibm-granite", "granite-docling-258m"),
    "granite-vision-4.1-4b": ("ibm-granite", "granite-vision-4.1-4b"),
    "granite-vision-3.3-2b-embedding": ("ibm-granite", "granite-vision-3.3-2b-embedding"),
    "granite-speech-4.1-2b": ("ibm-granite", "granite-speech-4.1-2b"),
    "granite-speech-4.1-2b-plus": ("ibm-granite", "granite-speech-4.1-2b-plus"),
    "nvidia-nemotron-parse-2.0": ("nvidia", "Nemotron-Parse-2.0"),
    "nvidia-nemotron-parse-v1.1": ("nvidia", "Nemotron-Parse-V1.1"),
    "nvidia-nemotron-parse-v1.2": ("nvidia", "Nemotron-Parse-V1.2"),
}

def run(cmd, timeout=25):
    try:
        r = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout + 10)
        return r.stdout
    except Exception:
        return None

def ctx_from(cfg):
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

for mid, (org, repo) in EXACT.items():
    if mid in fetched:
        continue
    url = f"https://huggingface.co/{org}/{repo}/resolve/main/config.json"
    body = run(["curl", "-sL", "-m", "25", url])
    if not body:
        print(f"{mid}: NO BODY ({org}/{repo})")
        continue
    try:
        cfg = json.loads(body)
    except Exception:
        print(f"{mid}: BAD JSON ({org}/{repo})")
        continue
    ctx = ctx_from(cfg)
    hs = cfg.get("hidden_size") or cfg.get("d_model")
    print(f"{mid}: ctx={ctx} hs={hs} ({org}/{repo})")
    if ctx:
        fetched[mid] = {"ctx": ctx, "hidden_size": hs, "url": url,
                        "fields": {k: cfg.get(k) for k in
                                   ("max_position_embeddings","n_positions",
                                    "model_max_length","hidden_size","d_model")}}

cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
missing_all = [mid for mid, x in cat.items() if x.get("context_window_tokens") is None]
failed = sorted(mid for mid in missing_all if mid not in fetched)
json.dump({"fetched": fetched, "failed": failed}, open(RES, "w"),
          ensure_ascii=False, indent=2)
print(f"\nTOTAL fetched {len(fetched)} / {len(failed)} failed of {len(missing_all)}")
print("failed:", ", ".join(failed))
