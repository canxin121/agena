"""Small stdlib-only Agena service-plugin runner; stdout is framed RPC only.

Manifests are generated from the Rust SDK, not a second hand-written contract.
The transport is Content-Length JSON-RPC, shared with the production SDK.
"""
from __future__ import annotations
import json
import sys
from pathlib import Path
from typing import Any, BinaryIO, Protocol

MAX_FRAME = 2 * 1024 * 1024

class Adapter(Protocol):
    def initialize(self, settings: dict[str, Any]) -> None: ...
    def describe(self) -> dict[str, Any]: ...
    def poll(self, request: dict[str, Any]) -> dict[str, Any]: ...
    def close(self) -> None: ...

def read_frame(stream: BinaryIO) -> dict[str, Any] | None:
    length: int | None = None
    used = 0
    while True:
        line = stream.readline(8193)
        if not line:
            return None
        used += len(line)
        if used > 8192:
            raise ValueError("RPC headers too large")
        if line in (b"\r\n", b"\n"):
            break
        key, sep, value = line.partition(b":")
        if not sep:
            raise ValueError("invalid RPC header")
        if key.lower() == b"content-length":
            if length is not None:
                raise ValueError("duplicate content length")
            length = int(value.strip())
    if length is None or not 0 <= length <= MAX_FRAME:
        raise ValueError("invalid RPC length")
    payload = stream.read(length)
    if len(payload) != length:
        raise ValueError("incomplete RPC body")
    result = json.loads(payload)
    if not isinstance(result, dict):
        raise ValueError("RPC object required")
    return result

def write_frame(value: dict[str, Any]) -> None:
    body = json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()
    if len(body) > MAX_FRAME:
        raise ValueError("RPC output too large")
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode() + body)
    sys.stdout.buffer.flush()

def serve(name: str, adapter: Adapter) -> None:
    definitions = json.loads(Path(__file__).with_name("manifests.json").read_text())
    manifest = definitions[name]
    initialized = False
    try:
        while (request := read_frame(sys.stdin.buffer)) is not None:
            call_id = request.get("id")
            method = request.get("method")
            params = request.get("params") or {}
            try:
                if method == "meta/manifest":
                    result: Any = manifest
                elif method == "meta/init":
                    context = params.get("context", params)
                    if context.get("protocol_version") != definitions["protocol_version"]:
                        raise ValueError("unsupported Agena protocol version")
                    settings = context.get("settings") or {}
                    if not isinstance(settings, dict):
                        raise ValueError("plugin settings must be an object")
                    if not initialized:
                        adapter.initialize(settings)
                        initialized = True
                    result = {"manifest": manifest, "protocol_version": definitions["protocol_version"]}
                elif method in {"meta/ping", "meta/host.rebind", "meta/host.fork", "meta/host.retire"}:
                    result = None
                elif method == "meta/shutdown":
                    if call_id is not None:
                        write_frame({"jsonrpc": "2.0", "id": call_id, "result": None})
                    break
                elif method == "service/invoke":
                    if not initialized or params.get("service") != "agena.integration" or params.get("api_version") != 1:
                        raise ValueError("unknown/uninitialized service")
                    operation, value = params.get("method"), params.get("input")
                    if operation == "describe":
                        result = adapter.describe()
                    elif operation == "poll" and isinstance(value, dict):
                        result = adapter.poll(value)
                    elif operation == "send" and isinstance(value, dict) and hasattr(adapter, "send"):
                        result = adapter.send(value)  # type: ignore[attr-defined]
                    else:
                        raise ValueError("unsupported adapter method")
                elif call_id is None:
                    continue
                else:
                    raise ValueError("unsupported RPC method")
                if call_id is not None:
                    write_frame({"jsonrpc": "2.0", "id": call_id, "result": result})
            except Exception as error:
                # Never serialize raw HTTP bodies, authorization values or
                # exception URLs from third parties into the host transcript.
                print(f"adapter operation failed: {type(error).__name__}", file=sys.stderr, flush=True)
                if call_id is not None:
                    write_frame({"jsonrpc": "2.0", "id": call_id, "error": {"code": -32603, "message": "Adapter operation failed; inspect the plugin configuration and local diagnostics."}})
    finally:
        adapter.close()
