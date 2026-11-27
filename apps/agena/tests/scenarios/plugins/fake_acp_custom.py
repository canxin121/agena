#!/usr/bin/env python3
"""Minimal standard ACP v1 peer for testing generic custom ACP agent registration."""

import json
import os
from pathlib import Path
import sys
import traceback
import urllib.request


def record_exception(*error):
    Path(".custom-acp-error").write_text("".join(traceback.format_exception(*error)))
    sys.__excepthook__(*error)


sys.excepthook = record_exception

assert sys.argv[1:] == ["--acp-stdio", "--profile", "smoke"], sys.argv
assert os.environ.get("CUSTOM_ACP_TOKEN") == "custom-secret-123", os.environ.get("CUSTOM_ACP_TOKEN")
assert not any(key.upper().startswith("AGENA_") for key in os.environ), [
    key for key in os.environ if key.upper().startswith("AGENA_")
]

client = urllib.request.build_opener(urllib.request.ProxyHandler({}))
mcp_session = None
mcp_url = None
mcp_headers = {}
provider_configured = False
provider_current = {"apiType": "openai", "baseUrl": "https://default.example.invalid"}
session_id = "custom-acp-session-1"


def send(**fields):
    print(json.dumps({"jsonrpc": "2.0", **fields}), flush=True)


def mcp(method, params, *, request_id=1):
    global mcp_session
    body = {"jsonrpc": "2.0", "method": method, "params": params}
    if request_id is not None:
        body["id"] = request_id
    headers = {
        "Accept": "application/json, text/event-stream",
        "Content-Type": "application/json",
        **mcp_headers,
    }
    if method != "initialize":
        headers["MCP-Protocol-Version"] = "2025-11-25"
        if mcp_session:
            headers["Mcp-Session-Id"] = mcp_session
    req = urllib.request.Request(mcp_url, data=json.dumps(body).encode(), headers=headers)
    with client.open(req, timeout=30) as response:
        if method == "initialize":
            mcp_session = response.headers["Mcp-Session-Id"]
        if request_id is None:
            return None
        payload = response.read().decode()
        if response.headers.get_content_type() == "text/event-stream":
            payload = next(
                line[5:].strip()
                for line in payload.splitlines()
                if line.startswith("data:") and line[5:].strip()
            )
        return json.loads(payload)


for line in sys.stdin:
    request = json.loads(line)
    identifier = request.get("id")
    method = request["method"]
    params = request.get("params") or {}

    if method == "initialize":
        if params.get("protocolVersion") == 2:
            send(id=identifier, result={"protocolVersion": 1})
            break
        assert params["protocolVersion"] == 1, params
        caps = params.get("clientCapabilities") or {}
        # Configured with client_tools: false -> no fs/terminal delegation advertised.
        assert caps.get("fs") == {"readTextFile": False, "writeTextFile": False}, caps
        assert caps.get("terminal") is False, caps
        send(
            id=identifier,
            result={
                "protocolVersion": 1,
                "agentCapabilities": {
                    "mcpCapabilities": {"http": True},
                    "providers": {},
                    "sessionCapabilities": {"resume": {}, "close": {}},
                },
                "authMethods": [
                    {"id": "cli-login", "name": "CLI Login", "type": "terminal", "args": ["login"]}
                ],
            },
        )
    elif method == "providers/list":
        send(
            id=identifier,
            result={
                "providers": [
                    {
                        "providerId": "main",
                        "supported": ["openai"],
                        "required": True,
                        "current": provider_current,
                    }
                ]
            },
        )
    elif method == "providers/set":
        assert params["providerId"] == "main", params
        assert params["apiType"] == "openai", params
        assert params["baseUrl"] == "https://custom-acp.example.invalid/v1", params
        provider_current = {"apiType": params["apiType"], "baseUrl": params["baseUrl"]}
        provider_configured = True
        send(id=identifier, result={})
    elif method == "session/new":
        assert provider_configured, "providers/set should run before session/new"
        meta = params.get("_meta") or {}
        assert "claudeCode" not in meta, meta
        assert "antigravity" not in meta, meta
        assert len(params["mcpServers"]) == 1, params["mcpServers"]
        bridge = params["mcpServers"][0]
        assert bridge["type"] == "http" and bridge["name"] == "agena", bridge
        mcp_url = bridge["url"]
        mcp_headers = {h["name"]: h["value"] for h in bridge["headers"]}
        send(id=identifier, result={"sessionId": session_id})
    elif method == "session/prompt":
        assert params["sessionId"] == session_id
        mcp(
            "initialize",
            {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "codex-custom", "version": "1.0"},
            },
        )
        mcp("notifications/initialized", {}, request_id=None)
        mcp("tools/call", {"name": "tools_help", "arguments": {"tool": "fs.read"}})
        read_res = mcp(
            "tools/call",
            {"name": "tools_call", "arguments": {"tool": "fs.read", "input": {"file_path": "acp-input.txt"}}},
        )["result"]
        content_text = read_res["content"][0]["text"]
        assert "acp callback content" in content_text, read_res
        send(
            method="session/update",
            params={
                "sessionId": session_id,
                "update": {
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "custom acp ok: acp callback content"},
                },
            },
        )
        send(id=identifier, result={"stopReason": "end_turn"})
    elif method == "session/close":
        send(id=identifier, result={})
    else:
        raise AssertionError(f"unexpected ACP method: {method}")
