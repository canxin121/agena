#!/usr/bin/python3
"""An isolated JSON-RPC daemon controlled entirely by test requests."""
import json
import os
import signal
import subprocess
import sys
import time

with open(os.environ["AGENA_TEST_STARTS"], "a", encoding="utf-8") as starts:
    starts.write(str(os.getpid()) + "\n")


def send(value):
    body = json.dumps(value).encode()
    sys.stdout.buffer.write(b"Content-Length: " + str(len(body)).encode() + b"\r\n\r\n" + body)
    sys.stdout.buffer.flush()


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
    method = request.get("method")
    params = request.get("params") or {}
    if method is None:
        with open(os.environ["AGENA_TEST_REPLIES"], "a", encoding="utf-8") as replies:
            replies.write(json.dumps(request) + "\n")
        continue
    if method == "test/exit":
        os._exit(params.get("code", 0))
    if method == "test/signal":
        os.kill(os.getpid(), signal.SIGTERM)
    if method == "test/close_stdout":
        os.close(1)
        time.sleep(params.get("delay_ms", 200) / 1000)
        os._exit(params.get("code", 0))
    if method == "test/malformed":
        sys.stdout.buffer.write(b"Content-Length: 1\r\n\r\n{")
        sys.stdout.buffer.flush()
        os._exit(0)
    if method == "test/descendant_stdout":
        subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"], stdin=subprocess.DEVNULL)
        os._exit(0)
    if method == "test/callback":
        send({"jsonrpc": "2.0", "id": 100, "method": "host/test", "params": {}})
    if method == "test/notification":
        send({"jsonrpc": "2.0", "method": "host/test", "params": {}})
    if method == "test/close_stdin":
        os.close(0)
    if method == "test/noisy_stderr":
        sys.stderr.buffer.write(b"x" * 16_383 + "😀".encode() + b"x" * 100_000 + b"\n\xff\xfe\n" + b"y" * 100_000 + b"\nstderr-finished\n")
        sys.stderr.buffer.flush()
    if method == "hooks/tool.invoke.stream":
        if "early_chunks" in params["input"]:
            for index in range(params["input"]["early_chunks"]):
                send({"jsonrpc": "2.0", "method": "tool.stream.chunk", "params": {"stream_id": "early", "text_delta": str(index)}})
            send({"jsonrpc": "2.0", "method": "tool.stream.end", "params": {"stream_id": "early", "title": "", "summary": "done", "output_text": "done"}})
            send({"jsonrpc": "2.0", "id": request["id"], "result": {"stream_id": "early"}})
            continue
        send({"jsonrpc": "2.0", "method": "tool.stream.chunk", "params": {"stream_id": "reused", "text_delta": "first"}})
        send({"jsonrpc": "2.0", "id": request["id"], "result": {"stream_id": "reused"}})
        if params["input"]["complete"]:
            send({"jsonrpc": "2.0", "method": "tool.stream.chunk", "params": {"stream_id": "reused", "text_delta": "second"}})
            send({"jsonrpc": "2.0", "method": "tool.stream.end", "params": {"stream_id": "reused", "title": "", "summary": "done", "output_text": "done"}})
        continue
    if method == "test/end_stream":
        send({"jsonrpc": "2.0", "method": "tool.stream.end", "params": {"stream_id": "reused", "title": "", "summary": "done", "output_text": "done"}})
    send({"jsonrpc": "2.0", "id": request["id"], "result": {"ok": True}})
    if method == "test/reply_then_exit":
        os._exit(0)
    if method == "test/close_stdin":
        time.sleep(30)
