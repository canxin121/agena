#!/usr/bin/env python3
import urllib.request, re, html

def fetch(url):
    try:
        req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
        r = urllib.request.urlopen(req, timeout=45)
        t = r.read().decode("utf-8", "replace")
        return r.status, len(t), t
    except Exception as e:
        return None, 0, f"ERROR {e}"

# Alternate slugs (build.nvidia.com historically uses underscores)
slugs = [
    "llama-3_3-nemotron-super-49b-v1",
    "llama-3_1-nemotron-ultra-253b-v1",
    "llama-3_1-nemotron-nano-8b-v1",
    "llama-3_1-nemotron-nano-vl-8b-v1",
    "nemotron-3-nano-omni-30b-a3b-reasoning",
    "nemotron-3-nano-omni-30b-a3b",
    "nvidia-nemotron-nano-9b-v2",
    "nemotron-nano-12b-v2-vl",
    "nemotron-cascade-2-30b-a3b",
    "openreasoning-nemotron-32b",
    "mistral-medium-3.5-128b",
    "mistral-small-4.119b-2603",
]
for s in slugs:
    url = f"https://build.nvidia.com/nvidia/{s}"
    st, sz, t = fetch(url)
    if isinstance(t, str) and t.startswith("ERROR"):
        print(f"{s}: {t}")
        continue
    hits = {k: t.count(k) for k in ["enable_thinking", "chat_template_kwargs",
                                    "reasoning_budget", "thinking_budget", "reasoning_content", "think"]}
    # look for a title marker to distinguish real page vs shell
    title = "???"
    mt = re.search(r"<title>(.*?)</title>", t)
    if mt:
        title = html.unescape(mt.group(1))[:70]
    print(f"{s}: status={st} size={sz} title={title!r} hits={hits}")
