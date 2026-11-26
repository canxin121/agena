#!/usr/bin/env python3
"""Exercise the legacy SSE and stdio MCP transports exposed over ACP."""

import json
import os
from pathlib import Path
from queue import Queue
import socket
import subprocess
import sys
from threading import Thread
import traceback
import urllib.error
import urllib.parse
import urllib.request


def failure(*args):
    Path(".acp-mcp-error").write_text("".join(traceback.format_exception(*args)))
    sys.__excepthook__(*args)


sys.excepthook = failure
transport = Path(".acp-transport").read_text()
version = int(Path(".acp-mcp-version").read_text())
opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
session = "native-mcp"


def send(**message):
    print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


def update(update_type, **fields):
    send(method="session/update", params={
        "sessionId": session,
        "update": {"sessionUpdate": update_type, **fields},
    })


def request_status(url, status, headers=None, payload=None):
    try:
        request = urllib.request.Request(url, data=payload, headers=headers or {})
        with opener.open(request, timeout=5) as response:
            assert response.status == status, response.status
    except urllib.error.HTTPError as error:
        with error:
            assert error.code == status, error.code


class Stdio:
    protocol = "2025-11-25"

    def __init__(self, descriptor, version):
        if version == 1:
            assert "type" not in descriptor
        else:
            assert descriptor["type"] == "stdio"
        assert Path(descriptor["command"]).is_absolute()
        arg, address = descriptor["args"]
        assert arg == "--internal-acp-mcp-stdio"
        host, port = address.rsplit(":", 1)
        assert host == "127.0.0.1"
        self.address = (host, int(port))
        with socket.create_connection(self.address, timeout=5) as unauthorized:
            unauthorized.sendall(b"0" * 32)
            assert unauthorized.recv(1) == b""
        self.process = subprocess.Popen(
            [descriptor["command"], *descriptor["args"]],
            env={**os.environ, **{item["name"]: item["value"] for item in descriptor["env"]}},
            stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        )
        self.messages = Queue()
        Thread(target=self.read, daemon=True).start()

    def read(self):
        for line in self.process.stdout:
            self.messages.put(json.loads(line))

    def send(self, message):
        self.process.stdin.write(json.dumps(message).encode() + b"\n")
        self.process.stdin.flush()

    def receive(self):
        return self.messages.get(timeout=8)

    def wait_revoked(self):
        # Stdin stays open while the server revokes access. The worker must exit.
        assert self.process.wait(timeout=5) == 0
        self.process.stdin.close()
        self.process.stdout.close()
        try:
            with socket.create_connection(self.address, timeout=1):
                raise AssertionError("old stdio grant still accepts connections")
        except ConnectionRefusedError:
            pass


class Sse:
    protocol = "2024-11-05"

    def __init__(self, descriptor):
        assert descriptor["type"] == "sse"
        url = descriptor["url"]
        assert urllib.parse.urlsplit(url).hostname == "127.0.0.1"
        self.headers = {item["name"]: item["value"] for item in descriptor["headers"]}
        request_status(url, 401)
        request_status(url, 403, {**self.headers, "Origin": "https://example.invalid"})
        self.stream = opener.open(urllib.request.Request(url, headers={**self.headers,
                                                                       "Accept": "text/event-stream"}), timeout=5)
        assert self.stream.headers.get_content_type() == "text/event-stream"
        event, path = self.event()
        assert event == "endpoint" and path.startswith("/messages?sessionId=")
        self.endpoint = urllib.parse.urljoin(url, path)
        request_status(self.endpoint, 401, {"Content-Type": "application/json"}, b"{}")

    def event(self):
        fields = {}
        while True:
            line = self.stream.readline()
            if not line:
                return None
            line = line.decode().rstrip("\r\n")
            if not line and fields:
                return fields["event"], fields["data"]
            if ":" in line and not line.startswith(":"):
                key, value = line.split(":", 1)
                fields[key] = value.lstrip()

    def send(self, message):
        request_status(self.endpoint, 202, {**self.headers, "Content-Type": "application/json"},
                       json.dumps(message).encode())

    def receive(self):
        name, message = self.event()
        assert name == "message"
        return json.loads(message)

    def wait_revoked(self):
        assert self.event() is None
        self.stream.close()


def verify_tools(client):
    def call(identifier, method, parameters=None):
        message = {"jsonrpc": "2.0", "method": method}
        if parameters is not None:
            message["params"] = parameters
        if identifier is not None:
            message["id"] = identifier
        client.send(message)
        if identifier is not None:
            response = client.receive()
            assert response["id"] == identifier and "result" in response, response
            return response["result"]

    info = call(1, "initialize", {
        "protocolVersion": client.protocol, "capabilities": {},
        "clientInfo": {"name": "acp-mcp-fixture", "version": "1"},
    })
    assert info["protocolVersion"] == client.protocol
    assert "tools" in info["capabilities"] and not {"resources", "prompts"} & info["capabilities"].keys()
    call(None, "notifications/initialized")
    tools = call(2, "tools/list")["tools"]
    assert any(tool["name"] == "fs_read" for tool in tools)
    shared = call(3, "tools/call", {"name": "fs_read", "arguments": {"file_path": "shared.txt"}})
    assert not shared.get("isError") and "expected MCP payload" in json.dumps(shared)
    denied = call(4, "tools/call", {"name": "fs_read", "arguments": {"file_path": "private.txt"}})
    assert denied.get("isError") and "withheld MCP payload" not in json.dumps(denied)


for line in sys.stdin:
    message = json.loads(line)
    method = message["method"]
    params = message["params"]
    identifier = message.get("id")
    if method == "initialize":
        if version == 1 and params["protocolVersion"] == 2:
            send(id=identifier, result={"protocolVersion": 1})
            break
        assert params["protocolVersion"] == version
        if version == 1:
            capabilities = {"mcpCapabilities": {"sse": True}} if transport == "sse" else {}
            send(id=identifier, result={"protocolVersion": 1, "agentCapabilities": capabilities})
        else:
            capabilities = {"mcp": {"stdio": {}}} if transport == "stdio" else {}
            send(id=identifier, result={"protocolVersion": 2, "info": {"name": "fixture", "version": "1"},
                                        "capabilities": {"session": capabilities}})
    elif method == "session/new":
        servers = params["mcpServers"]
        if transport == "none":
            assert not servers
        else:
            [descriptor] = servers
            assert descriptor["name"] == "agena"
        send(id=identifier, result={"sessionId": session})
    elif method == "session/prompt":
        assert params["sessionId"] == session
        if version == 2:
            send(id=identifier, result={"messageId": "user:fixture"})
            update("state_update", state="running")
        if transport != "none":
            client = Stdio(descriptor, version) if transport == "stdio" else Sse(descriptor)
            verify_tools(client)
        content = {"type": "text", "text": "transport verified"}
        if version == 2:
            update("agent_message", messageId="assistant:fixture", content=[content])
            update("state_update", state="idle", stopReason="end_turn")
        else:
            update("agent_message_chunk", content=content)
            send(id=identifier, result={"stopReason": "end_turn"})
        if transport != "none":
            client.wait_revoked()
            Path(".acp-grant-closed").touch()
    elif method == "session/close":
        send(id=identifier, result={})
