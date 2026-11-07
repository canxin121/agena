"""Owned newline JSON-RPC peer for Rust TestAgena system tests; no network."""
import json
import os
import signal
import sys

record_path = sys.argv[1]


def record(value):
    with open(record_path, "a", encoding="utf-8") as stream:
        stream.write(json.dumps(value, ensure_ascii=False) + "\n")
        stream.flush()


try:
    with open("/proc/self/stat", "r", encoding="utf-8") as stream:
        starttime = stream.read().rsplit(")", 1)[1].split()[19]
except FileNotFoundError:
    starttime = None

record({
    "event": "started",
    "pid": os.getpid(),
    "starttime": starttime,
    "argv": sys.argv[2:],
    "cwd": os.getcwd(),
    "token": os.environ.get("TOKEN"),
})


def stop(signum, _frame):
    record({"event": "stopped", "pid": os.getpid(), "signal": signum})
    sys.exit(0)


signal.signal(signal.SIGTERM, stop)
for request_number, line in enumerate(sys.stdin, start=1):
    if request_number > 256:
        record({"event": "request-budget-exceeded", "pid": os.getpid(), "limit": 256})
        sys.exit(2)
    request = json.loads(line)
    record({"event": "request", "pid": os.getpid(), "request": request})
    if "id" not in request:
        continue
    method = request.get("method")
    params = request.get("params") or {}
    if method == "initialize":
        result = {
            "protocolVersion": "2025-11-25",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "testkit-stdio-peer", "version": "1.0"},
        }
    elif method == "tools/list":
        result = {"tools": [{
            "name": "echo",
            "description": "stdio-owned-echo accepts local fixture text",
            "inputSchema": {
                "type": "object",
                "properties": {"text": {"type": "string"}},
                "required": ["text"],
                "additionalProperties": False,
            },
            "annotations": {"readOnlyHint": True, "destructiveHint": False},
        }]}
    elif method == "tools/call" and params.get("name") == "echo":
        text = (params.get("arguments") or {}).get("text", "")
        result = {
            "content": [{"type": "text", "text": "stdio-peer:" + text}],
            "structuredContent": {"echo": text, "origin": "local-stdio"},
            "isError": False,
        }
    elif method == "ping":
        result = {}
    else:
        response = {
            "jsonrpc": "2.0", "id": request["id"],
            "error": {"code": -32601, "message": "Local peer does not support this method"},
        }
        print(json.dumps(response), flush=True)
        continue
    print(json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}), flush=True)
record({"event": "eof", "pid": os.getpid()})
