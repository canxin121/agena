#!/usr/bin/env python3
import urllib.request, re, html

def fetch(url):
    req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    return urllib.request.urlopen(req, timeout=45).read().decode("utf-8", "replace")

def clean(seg):
    seg = re.sub(r"<[^>]+>", "", seg)
    seg = html.unescape(seg)
    return re.sub(r"[ \t]+", " ", seg)

# rendered example on super-120b page: find a 'extra_body=' assignment with reasoning_budget
t = fetch("https://build.nvidia.com/nvidia/nemotron-3-super-120b-a12b")
i = t.find("reasoning_budget=16384")
print("super-120b rendered extra_body hits for 'reasoning_budget':", t.count("reasoning_budget"))
for m in re.finditer(r'extra_body=\{[^}]{0,200}reasoning_budget[^}]{0,120}\}', t):
    print("RENDERED:", clean(m.group(0))[:250])
    break
# also the JSON variant
for m in re.finditer(r'"reasoning_budget"\s*:\s*\d+', t):
    print("JSON hit:", clean(m.group(0)))
    break

# nano-30b page: rendered example
t2 = fetch("https://build.nvidia.com/nvidia/nemotron-3-nano-30b-a3b")
print("\nnano-30b-a3b: 'enable_thinking' rendered JSON occurrences:")
for m in re.finditer(r'.{0,80}"enable_thinking".{0,80}', t2):
    s = clean(m.group(0))
    if "extra_body" in s or "chat_template_kwargs" in s:
        print("  ...", s[:180])
