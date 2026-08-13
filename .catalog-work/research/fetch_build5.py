#!/usr/bin/env python3
import urllib.request, re, html

def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    return urllib.request.urlopen(req, timeout=45).read().decode("utf-8", "replace")

def clean(seg):
    seg = re.sub(r"<[^>]+>", "", seg)
    seg = html.unescape(seg)
    return re.sub(r"[ \t]+", " ", seg)

jobs = [
    # (name, url, keyword to locate the template logic)
    ("nemotron-3-nano-30b-a3b", "https://build.nvidia.com/nvidia/nemotron-3-nano-30b-a3b", "enable_thinking"),
    ("nemotron-3.5-lightning-30b-a3b", "https://build.nvidia.com/nvidia/nemotron-3.5-lightning-30b-a3b", "enable_thinking"),
    ("nvidia-nemotron-nano-9b-v2", "https://build.nvidia.com/nvidia/nvidia-nemotron-nano-9b-v2", "enable_thinking"),
    ("llama-3_3-nemotron-super-49b-v1", "https://build.nvidia.com/nvidia/llama-3_3-nemotron-super-49b-v1", "think"),
    ("llama-3_1-nemotron-ultra-253b-v1", "https://build.nvidia.com/nvidia/llama-3_1-nemotron-ultra-253b-v1", "think"),
    ("llama-3_1-nemotron-nano-8b-v1", "https://build.nvidia.com/nvidia/llama-3_1-nemotron-nano-8b-v1", "think"),
]
for name, url, kw in jobs:
    try:
        t = fetch(url)
    except Exception as e:
        print(f"===== {name}: FETCH ERROR {e}")
        continue
    print(f"===== {name} =====")
    # find all occurrences of kw in the JS code region and print context around the first few
    idx = 0
    shown = 0
    while shown < 3:
        i = t.find(kw, idx)
        if i < 0:
            break
        seg = t[max(0, i - 300):i + 300]
        # only print if this looks like code (contains 'extra_body' or 'message' or 'chat')
        if re.search(r"extra_body|chat\.completions|messages|reasoning", seg):
            print(f"-- hit @{i} --")
            print(clean(seg)[:600])
            print()
            shown += 1
        idx = i + len(kw)
    if shown == 0:
        print("  (no code-context occurrences found)")
