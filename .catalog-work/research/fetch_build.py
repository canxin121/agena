#!/usr/bin/env python3
import urllib.request, re, html
models = [
 "nemotron-3-super-120b-a12b",
 "nemotron-3-ultra-550b-a55b",
 "nemotron-3-nano-30b-a3b",
 "nemotron-3.5-lightning-30b-a3b",
 "nemotron-3-nano-omni",
 "llama-3.3-nemotron-super-49b-v1",
 "llama-3.1-nemotron-ultra-253b-v1",
 "llama-3.1-nemotron-nano-8b-v1",
 "llama-3.1-nemotron-nano-vl-8b-v1",
]
for m in models:
    url = f"https://build.nvidia.com/nvidia/{m}"
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
        t = urllib.request.urlopen(req, timeout=40).read().decode("utf-8", "replace")
    except Exception as e:
        print(f"{m}: FETCH ERROR {e}")
        continue
    found = {}
    for kw in ["enable_thinking", "chat_template_kwargs", "thinking_budget",
               "reasoning_budget", "thinking_token_budget", "reasoning_content"]:
        found[kw] = t.count(kw)
    print(f"{m}: {found}")
    for kw in ["chat_template_kwargs", "enable_thinking"]:
        i = t.find(kw)
        if i >= 0:
            snippet = re.sub(r"<[^>]+>", " ", t[max(0, i - 400):i + 400])
            snippet = html.unescape(re.sub(r"\s+", " ", snippet))
            print("   ...", snippet[:500])
            break
