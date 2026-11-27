#!/usr/bin/env python3
"""Check that ACP executes Agena tools with the original input and tool contracts."""

import json
from pathlib import Path
import sys
import traceback
import urllib.request


NATIVE_ID = "acp-input-hooks"
TURN_COUNT = Path(".acp-input-hook-turns")
turns = int(TURN_COUNT.read_text()) if TURN_COUNT.is_file() else 0


def report_failure(*error):
    Path(".acp-input-hook-error").write_text("".join(traceback.format_exception(*error)))


sys.excepthook = report_failure


def send(**message):
    print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


def check_agena_tools(descriptor):
    headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream"}
    headers.update((item["name"], item["value"]) for item in descriptor["headers"])
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def rpc(method, params, identifier):
        body = {"jsonrpc": "2.0", "method": method, "params": params}
        if identifier is not None:
            body["id"] = identifier
        request = urllib.request.Request(descriptor["url"], json.dumps(body).encode(), headers)
        with opener.open(request, timeout=30) as response:
            if "Mcp-Session-Id" in response.headers:
                headers["Mcp-Session-Id"] = response.headers["Mcp-Session-Id"]
            if identifier is None:
                return
            body = response.read().decode()
            if response.headers.get_content_type() == "text/event-stream":
                body = next(line[5:].strip() for line in body.splitlines()
                            if line.startswith("data:") and line[5:].strip())
        return json.loads(body)["result"]

    rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                       "clientInfo": {"name": "ACP hook fixture", "version": "1"}}, 1)
    headers["Mcp-Protocol-Version"] = "2025-11-25"
    rpc("notifications/initialized", {}, None)
    help_text = rpc("tools/call", {"name": "tools_help", "arguments": {
        "tool": "testkit.model-hooks.probe",
    }}, 2)["content"][0]["text"]
    assert "Return the plugin fixture marker" in help_text and "TOOL DEFINITION HOOK MARKER" not in help_text
    for identifier, tool, args, marker in [
        (3, "testkit.model-hooks.probe", {}, "ACP plugin tool executed"),
        (4, "shell.exec", {"command": "printf 'ACP shell executed'"}, "ACP shell executed"),
    ]:
        result = rpc("tools/call", {"name": "tools_call", "arguments": {
            "tool": tool, "input": args,
        }}, identifier)
        assert not result.get("isError", False) and marker in json.dumps(result), result


for line in sys.stdin:
    request = json.loads(line)
    method, params = request["method"], request["params"]
    if "id" not in request:
        continue
    result = {}
    if method == "initialize":
        result = {"protocolVersion": 1, "agentCapabilities": {
            "loadSession": True, "mcpCapabilities": {"http": True},
            "sessionCapabilities": {"resume": {}, "systemPrompt": {}},
        }}
    elif method in ("session/new", "session/resume", "session/load"):
        assert (method == "session/new") == (turns == 0), method
        assert params.get("sessionId", NATIVE_ID) == NATIVE_ID
        assert params["systemPrompt"]["mode"] == "append"
        text = params["systemPrompt"]["content"][0]["text"]
        assert "SYSTEM HOOK MARKER" not in text
        assert "Prefer the `agena` MCP server's tools" in text
        descriptor, = params["mcpServers"]
        if turns < 2:
            label = ["first", "second"][turns]
            assert f"ACP instruction {label}" in text, text
        assert params["_meta"]["claudeCode"]["options"] == {"allowedTools": ["mcp__agena__*"]}
        if method == "session/new":
            result = {"sessionId": NATIVE_ID}
    elif method == "session/prompt":
        assert params["sessionId"] == NATIVE_ID
        blocks = params["prompt"]
        if turns < 2:
            label = ["first", "second"][turns]
            assert blocks == [{"type": "text", "text": f"acp-hook-original-{label}"}], blocks
            if turns == 0:
                check_agena_tools(descriptor)
            answer = "ACP fixture received the original message"
        else:
            assert turns == 2 and blocks == [{"type": "text", "text": "/status"}], blocks
            answer = "ACP fixture kept the native command"
        turns += 1
        TURN_COUNT.write_text(str(turns))
        send(method="session/update", params={
            "sessionId": NATIVE_ID, "update": {"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": answer}},
        })
        result = {"stopReason": "end_turn"}
    elif method != "session/close":
        raise AssertionError(method)
    send(id=request["id"], result=result)
