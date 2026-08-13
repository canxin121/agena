#!/usr/bin/env python3
import urllib.request, re, html

def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    return urllib.request.urlopen(req, timeout=45).read().decode("utf-8", "replace")

def clean(seg):
    seg = re.sub(r"<[^>]+>", "", seg)
    seg = html.unescape(seg)
    return re.sub(r"[ \t]+", " ", seg)

targets = [
    ("nemotron-3-ultra-550b-a55b", "https://build.nvidia.com/nvidia/nemotron-3-ultra-550b-a55b"),
    ("nemotron-3-nano-30b-a3b", "https://build.nvidia.com/nvidia/nemotron-3-nano-30b-a3b"),
    ("nemotron-3.5-lightning-30b-a3b", "https://build.nvidia.com/nvidia/nemotron-3.5-lightning-30b-a3b"),
    ("nemotron-3-nano-omni-30b-a3b-reasoning", "https://build.nvidia.com/nvidia/nemotron-3-nano-omni-30b-a3b-reasoning"),
    ("nvidia-nemotron-nano-9b-v2", "https://build.nvidia.com/nvidia/nvidia-nemotron-nano-9b-v2"),
]
for name, url in targets:
    try:
        t = fetch(url)
    except Exception as e:
        print(f"===== {name}: FETCH ERROR {e}")
        continue
    print(f"===== {name} =====")
    # Extract the reasoning_effort logic block (JS template)
    i = t.find("reasoning_effort")
    if i >= 0:
        seg = t[max(0, i - 1200):i + 1200]
        print("-- reasoning_effort logic --")
        print(clean(seg)[:2200])
    else:
        print("-- no reasoning_effort --")
    # For nano-9b: extract thinking_budget block
    i = t.find("thinking_budget")
    if i >= 0:
        seg = t[max(0, i - 1200):i + 800]
        print("-- thinking_budget context --")
        print(clean(seg)[:1800])
    # Extract chat.completions.create example
    i = t.find("chat.completions.create")
    if i >= 0:
        seg = t[i - 200:i + 700]
        print("-- example --")
        print(clean(seg)[:900])
    print()
