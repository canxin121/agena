#!/usr/bin/env python3
import urllib.request, json, html

def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    return urllib.request.urlopen(req, timeout=45).read().decode("utf-8", "replace")

def extract_playground(t):
    i = t.find('"playground"')
    if i < 0:
        return None
    j = t.find('{', i)
    if j < 0:
        return None
    depth = 0
    in_str = False
    esc = False
    k = j
    while k < len(t):
        c = t[k]
        if in_str:
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = False
        else:
            if c == '"':
                in_str = True
            elif c == "{":
                depth += 1
            elif c == "}":
                depth -= 1
                if depth == 0:
                    return t[j:k + 1]
        k += 1
    return None

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
    ("mistral-small-4.119b-2603", "https://build.nvidia.com/nvidia/mistral-small-4.119b-2603"),
]
for name, url in pages:
    try:
        t = fetch(url)
    except Exception as e:
        print(f"{name}: FETCH ERROR {e}")
        continue
    raw = extract_playground(t)
    if raw is None:
        print(f"{name}: no playground config found")
        continue
    try:
        cfg = json.loads(raw)
    except Exception as e:
        print(f"{name}: unparseable ({e}) len={len(raw)}")
        continue
    # normalize: json.loads unescapes
    print(f"=== {name} ===")
    print(json.dumps(cfg, indent=1)[:1100])
    print()
