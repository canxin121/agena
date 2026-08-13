#!/usr/bin/env python3
"""Build GitHub contents-API PUT body for models.json from the merged catalog.
Usage: build_push_body.py <sha> <out.json> <message>"""
import base64, json, sys

sha, out, msg = sys.argv[1], sys.argv[2], sys.argv[3]
content = base64.b64encode(open("models.merged.json", "rb").read()).decode()
body = {"message": msg, "content": content, "sha": sha, "branch": "main"}
json.dump(body, open(out, "w"))
print(f"wrote {out} ({len(content)} b64 chars)")
