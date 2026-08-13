#!/usr/bin/env python3
"""Check catalog coverage of key recent (2026) domestic releases."""
import json, os

D = "/Volumes/Rc20/Projects/agena/.claude/worktrees/github-model-catalog/.catalog-work"
cat = json.load(open(os.path.join(D, "models.merged.json")))["models"]
ids = set(cat.keys())

RECENT = [
    # deepseek dated variants
    "deepseek-v4-flash-0731", "deepseek-v4-pro-0813", "deepseek-v3.2", "deepseek-3.2",
    "deepseek-v3.1", "deepseek-r1-0528", "deepseek-v4-latest",
    # glm 5.x
    "glm-5", "glm-5.1", "glm-5.2", "glm-5.2-fast", "glm-4.7", "glm-4.7-flash",
    "glm-4.7-n", "glm-4.6", "glm-4.6-turbo", "glm-5v", "glm-5v-turbo",
    # kimi
    "kimi-k2.6", "kimi-k2.6-flex", "kimi-k2.7", "kimi-k2.7-code", "kimi-k3",
    "kimi-k3-fast", "kimi-k3-eco", "kimi-k3.256k", "kimi-k2.5",
    # minimax
    "minimax-m2.1", "minimax-m2.5", "minimax-m2.5-fast", "minimax-m3", "minimax-m1.80k",
    # mimo
    "mimo-v2.5", "mimo-v2.5-pro", "mimo-v2-flash",
    # ling
    "ling-3.0-flash", "ling-3.0-tiny", "ling-2.6-1t", "ling-2.6-flash",
    # doubao/seed dated
    "doubao-seed-2-0-pro", "doubao-seed-2-0-lite", "doubao-seed-2-0-code",
    "seed-2-0-code", "seed-2-1-turbo", "seed-2.0-code",
    # qwen latest
    "qwen3.5-max", "qwen-3-7-max", "qwen-3-8-max", "qwen3.6-max-preview",
    "qwen3-coder", "qwen3-coder-next", "qwen3.6-35b-fast",
    # hunyuan
    "hunyuan-2.0", "hunyuan-2.0-instruct", "hunyuan-2.0-thinking",
    # step / ernie / baichuan / baichuan4
    "step-3", "step-3-5-flash", "ernie-5.1", "ernie-x1.1-preview",
    "baichuan4-turbo", "baichuan4-air",
    # coding routers
    "coding-router", "coding-glm-5.1", "coding-minimax-m2.7",
    # misc
    "deepseek-ocr", "deepseek-ocr-2", "deepseek-math-v2", "autoglm-phone-9b",
]

print(f"{'model':50s} in-catalog?")
for m in RECENT:
    present = m in ids
    # fuzzy: also check stem match
    print(f"  {m:48s} {'YES' if present else 'NO '}")
