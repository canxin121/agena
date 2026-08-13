#!/usr/bin/env python3
import urllib.request, json

url = "https://build.nvidia.com/nvidia/llama-3_3-nemotron-super-49b-v1"
req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
t = urllib.request.urlopen(req, timeout=45).read().decode("utf-8", "replace")

i = t.find('\\"playground\\":')
print("key at", i)
seg = t[i:i + 40]
print("after key:", repr(seg))
j = t.find('{', i)
print("brace at", j, "diff", j - i)
# brace count from j
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
                raw = t[j:k + 1]
                print("closed at", k, "rawlen", len(raw))
                unesc = raw.replace('\\"', '"').replace('\\\\', '\\')
                try:
                    cfg = json.loads(unesc)
                    print("PARSED OK")
                    print(json.dumps(cfg)[:800])
                except Exception as e:
                    print("parse error:", e)
                break
    k += 1
else:
    print("never closed")
