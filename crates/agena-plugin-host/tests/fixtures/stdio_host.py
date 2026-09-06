#!/usr/bin/python3
"""A real plugin that records initialization and routes calls through its host."""
import json
import os
import sys
import time

with open(os.environ["AGENA_TEST_STARTS"], "a", encoding="utf-8") as starts:
    starts.write(str(os.getpid()) + "\n")

manifest = json.loads(os.environ["AGENA_TEST_MANIFEST"])
control_path = os.environ["AGENA_TEST_CONTROL"]
control = json.load(open(control_path, encoding="utf-8")) if os.path.exists(control_path) else {}
if "manifest_version" in control:
    manifest["version"] = control["manifest_version"]
initialized = False
init_context = None
init_host = None
callback_id = 1_000_000


def send(value):
    body = json.dumps(value).encode()
    sys.stdout.buffer.write(b"Content-Length: " + str(len(body)).encode() + b"\r\n\r\n" + body)
    sys.stdout.buffer.flush()


def read():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            sys.exit(0)
        if line == b"\r\n":
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1])
    return json.loads(sys.stdin.buffer.read(length))


def host_call(method, params):
    global callback_id
    callback_id += 1
    send({"jsonrpc": "2.0", "id": callback_id, "method": method, "params": params})
    while True:
        response = read()
        if response.get("id") == callback_id and "method" not in response:
            if "error" in response:
                raise RuntimeError(json.dumps(response["error"]))
            return response["result"]
        dispatch(response)


def dispatch(request):
    global initialized, init_context, init_host
    method = request["method"]
    params = request.get("params") or {}
    if method == "test/exit":
        os._exit(params.get("code", 7))
    if method == "meta/manifest":
        result = manifest
    elif method == "meta/init":
        if initialized:
            raise RuntimeError("meta/init cannot be called twice in one process")
        with open(os.environ["AGENA_TEST_EVENTS"], "a", encoding="utf-8") as events:
            events.write(json.dumps({"event": "initializing", "pid": os.getpid(), "context": request.get("context")}) + "\n")
        time.sleep(control.get("init_delay_ms", 0) / 1000)
        while "init_gate_file" in control and not os.path.exists(control["init_gate_file"]):
            time.sleep(0.005)
        if control.get("init_failure"):
            raise RuntimeError("injected initialization failure")
        init_host = host_call("host/config.read", {"context": request.get("context")})
        if "init_tool_name" in control:
            host_call("host/tool.registry.register", {"request": {"tool": {"name": control["init_tool_name"], "docs": {"summary": "dynamic"}, "contract": {"input_schema": {"type": "object"}}}}})
        init_context = params
        initialized = True
        result = {"manifest": manifest, "protocol_version": control.get("protocol_version", params["protocol_version"])}
    elif method == "test/state":
        result = {"initialized": initialized, "pid": os.getpid(), "context": init_context, "host": init_host}
    elif method == "test/host":
        result = host_call("host/config.read", {})
    elif method == "test/callback":
        result = host_call(params["method"], params.get("params", {}))
    elif method == "test/wait_then_register":
        host_call("host/config.read", {"path": "test.wait"})
        result = host_call("host/tool.registry.register", {"request": {"tool": {"name": "late", "contract": {"input_schema": {"type": "object"}}}}})
    elif method == "hooks/tool.invoke":
        callback = params["input"]
        callback_params = callback.get("params", {}).copy()
        callback_params["context"] = request.get("context")
        result = {"title": "callback", "summary": "callback", "output_text": json.dumps(host_call(callback["method"], callback_params))}
    elif method == "test/register":
        result = host_call("host/tool.registry.register", {"request": {"tool": {"name": params["name"], "docs": {"summary": "dynamic"}, "contract": {"input_schema": {"type": "object", "additionalProperties": False}}}}})
    else:
        result = {"ok": True}
    if "id" in request:
        send({"jsonrpc": "2.0", "id": request["id"], "result": result})


while True:
    request = read()
    try:
        dispatch(request)
    except Exception as error:
        send({"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32000, "message": str(error)}})
