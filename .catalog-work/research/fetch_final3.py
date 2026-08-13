#!/usr/bin/env python3
import urllib.request, re, html

def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    return urllib.request.urlopen(req, timeout=45).read().decode("utf-8", "replace")

def clean(seg):
    seg = re.sub(r"<[^>]+>", "", seg)
    seg = html.unescape(seg)
    return re.sub(r"[ \t]+", " ", seg)

# nano-9b-v2: full example around min_thinking_tokens
t = fetch("https://build.nvidia.com/nvidia/nvidia-nemotron-nano-9b-v2")
i = t.find("min_thinking_tokens")
seg = t[max(0, i - 2000):i + 1500]
print("===== nano-9b-v2 example region =====")
print(clean(seg)[:2600])
print()

# omni-reasoning: full example around enable_thinking
t2 = fetch("https://build.nvidia.com/nvidia/nemotron-3-nano-omni-30b-a3b-reasoning")
i2 = t2.find("chat_template_kwargs")
seg2 = t2[max(0, i2 - 1500):i2 + 1200]
print("===== omni-reasoning example region =====")
print(clean(seg2)[:2400])
