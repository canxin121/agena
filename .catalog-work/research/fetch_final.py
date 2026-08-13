#!/usr/bin/env python3
import urllib.request, re

def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    return urllib.request.urlopen(req, timeout=45).read().decode("utf-8", "replace")

def grab(win, pattern):
    m = re.search(pattern, win)
    return m.group(1) if m else None

pages = [
    ("llama-3.2-1b-instruct (meta)", "https://build.nvidia.com/meta/llama-3.2-1b-instruct"),
    ("mistral-medium-3.5-128b", "https://build.nvidia.com/nvidia/mistral-medium-3.5-128b"),
    ("llama-3.1-nemotron-nano-vl-8b-v1", "https://build.nvidia.com/nvidia/llama-3.1-nemotron-nano-vl-8b-v1"),
    ("nemotron-nano-12b-v2-vl", "https://build.nvidia.com/nvidia/nemotron-nano-12b-v2-vl"),
]
for name, url in pages:
    try:
        t = fetch(url)
    except Exception as e:
        print(f"{name}: FETCH ERROR {e}")
        continue
    i = t.find('\\"playground\\":')
    win = t[i:i + 4000] if i >= 0 else ""
    d = grab(win, r'\\"defaultEnabled\\":(true|false)')
    on = grab(win, r'\\"systemPromptEnabled\\":\\"([^\\"]*)\\"')
    off = grab(win, r'\\"systemPromptDisabled\\":\\"([^\\"]*)\\"')
    show = grab(win, r'\\"showReasoningToggle\\":(true|false)')
    print(f"{name}: defaultEnabled={d} showToggle={show} sysOn={on!r} sysOff={off!r}")
    # show any reasoning mention in window
    j = win.find("reasoning")
    print("  reasoning window snippet:", (win[j - 60:j + 250] if j >= 0 else "none")[:300].replace('\\"', '"'))
    print()
