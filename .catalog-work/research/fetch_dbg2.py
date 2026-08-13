#!/usr/bin/env python3
import urllib.request

url = "https://build.nvidia.com/nvidia/llama-3_3-nemotron-super-49b-v1"
req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
r = urllib.request.urlopen(req, timeout=45)
t = r.read().decode("utf-8", "replace")
print("len:", len(t))
print("escaped key count:", t.count('\\"playground\\":'))
i = 0
for _ in range(9):
    i = t.find("playground", i)
    if i < 0:
        break
    print(f"occ@{i}:", repr(t[i - 30:i + 60]))
    i += 1
