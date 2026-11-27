#!/usr/bin/env python3
"""Exercise the HTTP, legacy SSE and stdio MCP transports exposed over ACP."""

import json
import os
from pathlib import Path
from queue import Queue
import socket
import subprocess
import sys
from threading import Thread
import time
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
antigravity = Path(sys.argv[0]).name == "agy"

if antigravity:
    assert sys.argv[1:] == ["acp", "--process-per-session"], sys.argv


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


class Http:
    protocol = "2025-11-25"

    def __init__(self, descriptor):
        assert descriptor["type"] == "http"
        self.url = descriptor["url"]
        assert urllib.parse.urlsplit(self.url).hostname == "127.0.0.1"
        self.headers = {item["name"]: item["value"] for item in descriptor["headers"]}
        self.headers.update({
            "Content-Type": "application/json",
            "Accept": "application/json, text/event-stream",
            "MCP-Protocol-Version": self.protocol,
        })

    def request(self, message):
        return urllib.request.Request(self.url, data=json.dumps(message).encode(), headers=self.headers)

    def send(self, message):
        with opener.open(self.request(message), timeout=8) as response:
            if message["method"] == "initialize":
                session = response.headers.get("Mcp-Session-Id")
                assert session, "legacy Streamable HTTP must associate cancellation with the MCP session"
                self.headers["Mcp-Session-Id"] = session
            if "id" not in message:
                assert response.status == 202, response.status
                return
            content = response.read().decode()
            if response.headers.get_content_type() == "text/event-stream":
                content = next(line[5:].strip() for line in content.splitlines()
                               if line.startswith("data:") and line[5:].strip())
            self.result = json.loads(content)

    def receive(self):
        return self.result

    def wait_revoked(self):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            try:
                self.send({"jsonrpc": "2.0", "id": 8, "method": "tools/list"})
            except urllib.error.HTTPError as error:
                with error:
                    assert error.code == 401, error.code
                    return
            except urllib.error.URLError as error:
                assert isinstance(error.reason, ConnectionRefusedError), error
                return
            time.sleep(0.02)
        raise AssertionError("old HTTP grant still accepts connections")


def verify_tools(client, disconnect):
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
    assert {tool["name"] for tool in tools} == {"plugins_list", "tools_list", "tools_help", "tools_call"}
    plugins = call(10, "tools/call", {"name": "plugins_list", "arguments": {"plugin": "agena.fs"}})
    assert "agena.fs" in plugins["content"][0]["text"], plugins
    listed = call(5, "tools/call", {"name": "tools_list", "arguments": {"plugin": "agena.fs"}})
    assert "fs.read" in listed["content"][0]["text"], listed
    described = call(6, "tools/call", {"name": "tools_help", "arguments": {"tool": "fs.read"}})
    assert "file_path" in described["content"][0]["text"], described

    def agena_tool(tool, arguments):
        return {"name": "tools_call", "arguments": {"tool": tool, "input": arguments}}

    def read(identifier, path):
        return call(identifier, "tools/call", agena_tool("fs.read", json.dumps({"file_path": path})))

    shared = read(3, "shared.txt")
    assert not shared.get("isError") and "expected MCP payload" in json.dumps(shared)
    denied = read(4, "private.txt")
    assert denied.get("isError") and "withheld MCP payload" not in json.dumps(denied)
    if version == 2 and transport in ("http", "stdio"):
        result = call(7, "tools/call", agena_tool("shell.exec", {
            "command": "sleep 5 && printf 'retained MCP progress'",
        }))
        assert not result.get("isError") and "retained MCP progress" in json.dumps(result), result
        time.sleep(1.2)
    if version == 2 and transport in ("http", "stdio"):
        pending = {"jsonrpc": "2.0", "id": 8, "method": "tools/call", "params":
            agena_tool("shell.exec", {
                "command": "touch .mcp-shell-started; while [ ! -e .mcp-shell-release ]; do sleep .05; done; touch .mcp-shell-effect",
                "reads": [".mcp-shell-release"],
                "writes": [".mcp-shell-started", ".mcp-shell-effect"],
            })
        }
        worker = None
        errors = Queue()
        if transport == "http":
            def collect_response():
                try:
                    with opener.open(client.request(pending), timeout=20) as response:
                        response.read()
                except BaseException as error:
                    errors.put(error)

            worker = Thread(target=collect_response, daemon=True)
            worker.start()
        else:
            client.send(pending)
        for marker in (".mcp-shell-started", ".mcp-shell-release"):
            deadline = time.monotonic() + 20
            while not Path(marker).is_file():
                assert time.monotonic() < deadline, marker
                time.sleep(0.02)
            if marker == ".mcp-shell-started":
                if disconnect:
                    sys.exit(0)
                call(None, "notifications/cancelled", {"requestId": 8})
        assert not read(9, "shared.txt").get("isError"), "cancellation closed the MCP connection"
        if worker is not None:
            worker.join(timeout=5)
            assert not worker.is_alive(), "cancelled MCP request still has an open HTTP response"
            assert errors.empty(), errors.get_nowait() if not errors.empty() else None


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
            if antigravity:
                assert params["clientCapabilities"]["fs"] == {"readTextFile": False, "writeTextFile": False}
                assert params["clientCapabilities"]["terminal"] is False
            capabilities = {"mcpCapabilities": {"sse": True}} if transport == "sse" else {}
            send(id=identifier, result={"protocolVersion": 1, "agentCapabilities": capabilities})
        else:
            capabilities = {} if transport == "none" else {"mcp": {transport: {}}}
            send(id=identifier, result={"protocolVersion": 2, "info": {"name": "fixture", "version": "1"},
                                        "capabilities": {"session": capabilities}})
    elif method == "session/new":
        servers = params["mcpServers"]
        if antigravity:
            assert params["_meta"]["antigravity"]["eagerMcpServers"] == ["agena"]
            assert servers[0]["_meta"]["antigravity"]["forceAllToolsEager"] is True
        else:
            if servers:
                assert params["_meta"]["claudeCode"]["options"] == {"allowedTools": ["mcp__agena__*"]}
            else:
                assert "_meta" not in params
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
            client = Stdio(descriptor, version) if transport == "stdio" else {
                "sse": Sse,
                "http": Http,
            }[transport](descriptor)
            verify_tools(client, params["prompt"][0]["text"] == "disconnect during MCP")
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
