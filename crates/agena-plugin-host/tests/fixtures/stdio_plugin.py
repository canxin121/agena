#!/usr/bin/python3
"""Local stdio fixture; the parent supplies every path and manifest."""
import json
import os
import sys

with open(os.environ["AGENA_TEST_STARTS"], "a", encoding="utf-8") as starts:
    starts.write("started\n")

while True:
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line == b"\r\n":
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1])
    request = json.loads(sys.stdin.buffer.read(length))
    method = request["method"]
    if method == "test/exit":
        sys.exit(1)
    if method == "meta/manifest":
        result = json.loads(os.environ["AGENA_TEST_MANIFEST"])
    else:
        result = {"ok": True}
    response = json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}).encode()
    sys.stdout.buffer.write(b"Content-Length: " + str(len(response)).encode() + b"\r\n\r\n")
    sys.stdout.buffer.write(response)
    sys.stdout.buffer.flush()
