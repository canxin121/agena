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

# 1) Check zero-hit pages: correct slug? status? size?
for m in ["llama-3.3-nemotron-super-49b-v1", "llama-3.1-nemotron-ultra-253b-v1",
          "llama-3.1-nemotron-nano-8b-v1", "llama-3.1-nemotron-nano-vl-8b-v1",
          "nemotron-3-nano-omni", "llama-3.2-1b-instruct"]:
    url = f"https://build.nvidia.com/nvidia/{m}"
    st, sz, t = fetch(url)
    if isinstance(t, str) and t.startswith("ERROR"):
        print(f"{m}: {t}")
        continue
    hits = {k: t.count(k) for k in ["enable_thinking", "chat_template_kwargs",
                                    "reasoning_budget", "thinking_budget", "reasoning_content", "think"]}
    print(f"{m}: status={st} size={sz} hits={hits}")

# 2) Extract the full python chat-completions example for nemotron-3-super-120b-a12b
print("\n===== FULL PYTHON EXAMPLE: nemotron-3-super-120b-a12b =====")
st, sz, t = fetch("https://build.nvidia.com/nvidia/nemotron-3-super-120b-a12b")
i = t.find("chat.completions.create")
if i >= 0:
    seg = t[max(0, i - 900):i + 900]
    seg = re.sub(r"<[^>]+>", "", seg)
    seg = html.unescape(seg)
    print(seg)
