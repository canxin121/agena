#!/usr/bin/env python3
import urllib.request

url = "https://build.nvidia.com/nvidia/llama-3_3-nemotron-super-49b-v1"
req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
r = urllib.request.urlopen(req, timeout=45)
t = r.read().decode("utf-8", "replace")
print("status:", r.status, "len:", len(t))
print("'playground' count:", t.count("playground"))
i = t.find("playground")
print("first index:", i)
if i >= 0:
    print("context:", repr(t[i - 80:i + 120]))
