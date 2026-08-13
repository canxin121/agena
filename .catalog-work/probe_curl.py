#!/usr/bin/env python3
"""Test curl reachability of Chinese doc domains + pull HF README cards for
open-weight domestic models to extract knowledge-cutoff dates."""
import subprocess, json

def curl(url, timeout=20, maxlen=400):
    try:
        r = subprocess.run(["curl", "-sL", "-m", str(timeout), url],
                           capture_output=True, text=True, timeout=timeout + 5)
        body = r.stdout or r.stderr
        return body[:maxlen].replace("\n", " ")
    except Exception as e:
        return f"ERR {e}"

# 1. Chinese doc domains (blocked for WebFetch, test curl)
print("== Chinese doc domains via curl ==")
for name, url in [
    ("deepseek pricing", "https://api-docs.deepseek.com/quick_start/pricing"),
    ("zai models", "https://docs.z.ai/models"),
    ("moonshot pricing", "https://platform.moonshot.cn/docs/pricing/chat"),
    ("volcengine doubao", "https://www.volcengine.com/docs/82379/1263482"),
    ("deepseek homepage", "https://www.deepseek.com/"),
    ("zai glm-4.7-n", "https://docs.z.ai/en/models/glm-4.7-n"),
]:
    print(f"  {name:20s} {curl(url)[:120]}")

# 2. HF READMEs for open-weight domestic gaps
print("\n== HF README knowledge-cutoff probes ==")
HF = {
    "deepseek-math-v2": "deepseek-ai/DeepSeek-Math-V2",
    "deepseek-r1-distill-llama-8b": "deepseek-ai/DeepSeek-R1-Distill-Llama-8B",
    "deepseek-r1-distill-qwen-7b": "deepseek-ai/DeepSeek-R1-Distill-Qwen-7B",
    "deepseek-r1-distill-qwen-14b": "deepseek-ai/DeepSeek-R1-Distill-Qwen-14B",
    "deepseek-r1-distill-qwen-1-5b": "deepseek-ai/DeepSeek-R1-Distill-Qwen-1.5B",
    "qwen3guard-gen-8b": "Qwen/Qwen3-Guard-8B",
    "bge-multilingual-gemma2": "BAAI/bge-multilingual-gemma2",
    "glm-4-9b": "THUDM/glm-4-9b",
    "glm-4": "THUDM/glm-4-9b-chat",
    "yi-large": "01-ai/Yi-Large",
}
for mid, repo in HF.items():
    body = curl(f"https://huggingface.co/{repo}/raw/main/README.md", maxlen=600)
    print(f"  {mid:32s} {body[:200]}")
