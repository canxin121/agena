"""Local stdio LSP test peer. Never connects to the network or reads source files."""
import json
import os
import sys

root = None
while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line == b"\r\n":
            break
        key, value = line.decode("ascii").split(":", 1)
        headers[key.lower()] = value.strip()
    body = sys.stdin.buffer.read(int(headers["content-length"]))
    request = json.loads(body)
    method = request.get("method")
    if method == "exit":
        sys.exit(0)
    if "id" not in request:
        continue
    result = None
    if method == "initialize":
        root = request["params"].get("rootUri")
        with open(os.environ["AGENA_LSP_FIXTURE_LOG"], "a", encoding="utf-8") as log:
            log.write(json.dumps({"root": root, "cwd": os.getcwd(), "pid": os.getpid()}) + "\n")
        result = {"capabilities": {"hoverProvider": True}}
    elif method == "textDocument/hover":
        result = {"contents": root}
    response = json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(response)}\r\n\r\n".encode() + response)
    sys.stdout.buffer.flush()
