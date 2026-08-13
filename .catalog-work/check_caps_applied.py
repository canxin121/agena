#!/usr/bin/env python3
"""Spot-check applied caps for representative filled models + confirm the 28
remaining unset are all special niches."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]

SPOT = ["codellama-34b", "codellama-70b-instruct-hf", "llama2-7b-chat-hf",
        "gemma-3-1b-it", "mistral-large-2-instruct", "jamba-1.7-large-instruct",
        "jamba-1.5-large-instruct", "llama-3.2-nv-embedqa-1b-v1",
        "nv-embedqa-e5-v5", "palmyra-med-70b", "llama-3.1-nemotron-51b-instruct",
        "nemotron-nano-3-30b-a3b", "snowflake-arctic-embed-l-v2.0",
        "meta-llama-guard-2-8b", "llama-3.1-nemotron-ultra-253b-cpt-v1",
        "mistral-nemo-minitron-8b-8k-instruct", "mamba-codestral-7b-v0.1",
        "sea-lion-7b-instruct"]
print("SPOT CHECK (ctx / in / out):")
for m in SPOT:
    e = cat[m]
    print(f"  {m:48s} {e.get('context_window_tokens')} / {e.get('max_input_tokens')} / {e.get('max_output_tokens')}")

unset = [m for m, x in cat.items() if x.get("context_window_tokens") is None]
print(f"\nSTILL UNSET ({len(unset)}):")
for m in sorted(unset):
    print("  ", m)
