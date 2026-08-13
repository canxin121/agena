#!/usr/bin/env python3
import urllib.request

def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    r = urllib.request.urlopen(req, timeout=45)
    t = r.read().decode("utf-8", "replace")
    mt = None
    import re
    m = re.search(r"<title>(.*?)</title>", t)
    if m:
        mt = m.group(1)
    return r.status, len(t), mt

candidates = [
    ("meta/llama-3.2-1b-instruct", "https://build.nvidia.com/meta/llama-3.2-1b-instruct"),
    ("nvidia/llama-3.2-1b-instruct", "https://build.nvidia.com/nvidia/llama-3.2-1b-instruct"),
    ("openreasoning/nemotron-32b", "https://build.nvidia.com/openreasoning/nemotron-32b"),
    ("nvidia/nemotron-cascade-2-30b-a3b", "https://build.nvidia.com/nvidia/nemotron-cascade-2-30b-a3b"),
    ("nvidia/nemotron-cascade-2-30b-a3b:thinking", "https://build.nvidia.com/nvidia/nemotron-cascade-2-30b-a3b%3Athinking"),
    ("nvidia/mistral-small-4.119b-2603", "https://build.nvidia.com/nvidia/mistral-small-4.119b-2603"),
    ("nvidia/mistral-medium-3.5-128b:thinking", "https://build.nvidia.com/nvidia/mistral-medium-3.5-128b%3Athinking"),
    ("nvidia/nemotron-3-nano-30b-a3b:thinking", "https://build.nvidia.com/nvidia/nemotron-3-nano-30b-a3b%3Athinking"),
    ("nvidia/nemotron-3-nano-omni", "https://build.nvidia.com/nvidia/nemotron-3-nano-omni"),
]
for label, url in candidates:
    try:
        st, sz, title = fetch(url)
        import re
        # detect if real page (contains code chunks)
        req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
        t = urllib.request.urlopen(req, timeout=45).read().decode("utf-8", "replace")
        hits = {k: t.count(k) for k in ["enable_thinking", "chat_template_kwargs", "reasoning_budget", "thinking_budget"]}
        print(f"{label}: status={st} size={sz} title={title!r} hits={hits}")
    except Exception as e:
        print(f"{label}: ERROR {e}")
