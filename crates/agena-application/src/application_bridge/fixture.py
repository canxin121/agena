"""A framed stdio fixture that exercises the real authenticated host callback."""
import json
from pathlib import Path
import sys

root = Path(sys.argv[1])
definition = json.loads((root / "manifest.json").read_text())
sequence = 0


def read():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line in (b"\r\n", b"\n"):
            break
        key, value = line.split(b":", 1)
        if key.lower() == b"content-length":
            length = int(value)
    return json.loads(sys.stdin.buffer.read(length))


def write(value):
    body = json.dumps(value, separators=(",", ":")).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()


def callback(operation, context):
    global sequence
    sequence += 1
    call_id = f"fixture-{sequence}"
    write({"jsonrpc": "2.0", "id": call_id, "method": "host/session.op",
           "params": {"request": operation, "context": context or {}}})
    while True:
        response = read()
        if response is None:
            raise RuntimeError("host callback closed")
        if response.get("id") == call_id:
            return response
        handle(response)


def handle(request):
    method = request.get("method")
    if method is None:
        return
    params = request.get("params") or {}
    if method == "meta/manifest":
        result = definition["manifest"]
    elif method == "meta/init":
        result = definition
    elif method == "meta/ready":
        result = callback({"op": "query", "request": {"method": "health"}}, request.get("context"))
        (root / "ready.json").write_text(json.dumps(result))
        if "error" in result:
            write({"jsonrpc": "2.0", "id": request["id"], "error": result["error"]})
            return
        result = {}
    elif method == "command/invoke":
        operation = json.loads((root / "operation.json").read_text())
        output = callback(operation, request.get("context"))
        result = {"status": "succeeded", "title": "Host operation", "summary": "", "output": output}
    elif method in ("meta/ping", "meta/host.rebind", "meta/host.fork", "meta/host.retire", "meta/shutdown"):
        result = None
    else:
        if "id" in request:
            write({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32601, "message": "Unknown fixture method"}})
        return
    if "id" in request:
        write({"jsonrpc": "2.0", "id": request["id"], "result": result})


while (request := read()) is not None:
    handle(request)
    if request.get("method") == "meta/shutdown":
        break
