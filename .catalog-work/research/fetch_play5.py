#!/usr/bin/env python3
import urllib.request, re

def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    return urllib.request.urlopen(req, timeout=45).read().decode("utf-8", "replace")

pages = [
    ("llama-3.3-nemotron-super-49b-v1", "https://build.nvidia.com/nvidia/llama-3_3-nemotron-super-49b-v1"),
    ("llama-3.1-nemotron-ultra-253b-v1", "https://build.nvidia.com/nvidia/llama-3_1-nemotron-ultra-253b-v1"),
    ("llama-3.1-nemotron-nano-8b-v1", "https://build.nvidia.com/nvidia/llama-3_1-nemotron-nano-8b-v1"),
    ("llama-3.1-nemotron-nano-vl-8b-v1", "https://build.nvidia.com/nvidia/llama-3.1-nemotron-nano-vl-8b-v1"),
    ("nemotron-3-super-120b-a12b", "https://build.nvidia.com/nvidia/nemotron-3-super-120b-a12b"),
    ("nemotron-3-ultra-550b-a55b", "https://build.nvidia.com/nvidia/nemotron-3-ultra-550b-a55b"),
    ("nemotron-3-nano-30b-a3b", "https://build.nvidia.com/nvidia/nemotron-3-nano-30b-a3b"),
    ("nemotron-3.5-lightning-30b-a3b", "https://build.nvidia.com/nvidia/nemotron-3.5-lightning-30b-a3b"),
    ("nemotron-3-nano-omni-30b-a3b-reasoning", "https://build.nvidia.com/nvidia/nemotron-3-nano-omni-30b-a3b-reasoning"),
    ("nvidia-nemotron-nano-9b-v2", "https://build.nvidia.com/nvidia/nvidia-nemotron-nano-9b-v2"),
    ("nemotron-nano-12b-v2-vl", "https://build.nvidia.com/nvidia/nemotron-nano-12b-v2-vl"),
    ("mistral-medium-3.5-128b", "https://build.nvidia.com/nvidia/mistral-medium-3.5-128b"),
]
for name, url in pages:
    try:
        t = fetch(url)
    except Exception as e:
        print(f"{name}: FETCH ERROR {e}")
        continue
    i = t.find('\\"playground\\":')
    if i < 0:
        print(f"{name}: no playground")
        continue
    win = t[i:i + 3000]
    # regexes on escaped text
    def grab(pattern):
        m = re.search(pattern, win)
        return m.group(1) if m else None
    defaultEnabled = grab(r'\\"defaultEnabled\\":(true|false)')
    sysOn = grab(r'\\"systemPromptEnabled\\":\\"([^\\"]*)\\"')
    sysOff = grab(r'\\"systemPromptDisabled\\":\\"([^\\"]*)\\"')
    showToggle = grab(r'\\"showReasoningToggle\\":(true|false)')
    effort = grab(r'\\"reasoning_effort\\"')
    print(f"{name}: defaultEnabled={defaultEnabled} showToggle={showToggle} sysOn={sysOn!r} sysOff={sysOff!r} effort={effort}")
