#!/usr/bin/env python3
"""Endpoint configuration negotiation without external inference or credentials."""

import json
import os
from pathlib import Path
import sys
import traceback


def failure(*args):
    Path(".providers-error").write_text("".join(traceback.format_exception(*args)))
    sys.__excepthook__(*args)


sys.excepthook = failure
version = int(Path(".providers-version").read_text())
supported = not Path(".providers-unsupported").exists()
assert "AGENA_TEST_PROVIDER_HEADER" not in os.environ
assert "AGENA_ACP_PROVIDERS_FILE" not in os.environ
session = "provider-test-session"
main = {"apiType": "anthropic", "baseUrl": "https://native.example.invalid"}
optional = {"apiType": "openai", "baseUrl": "https://optional.example.invalid"}
listed = False
configured = False


def send(**payload):
    print(json.dumps({"jsonrpc": "2.0", **payload}), flush=True)


def update(update_type, **fields):
    send(method="session/update", params={
        "sessionId": session,
        "update": {"sessionUpdate": update_type, **fields},
    })


for raw in sys.stdin:
    message = json.loads(raw)
    method = message["method"]
    params = message["params"]
    identifier = message.get("id")
    with Path(".providers-wire").open("a") as trace:
        trace.write(method + "\n")

    if method == "initialize":
        if version == 1 and params["protocolVersion"] == 2:
            send(id=identifier, result={"protocolVersion": 1})
            break
        assert params["protocolVersion"] == version
        if version == 2:
            result = {"protocolVersion": 2, "info": {"name": "provider-fixture", "version": "1"},
                      "capabilities": {"session": {"options": {}, "mcp": {"http": {}}}}}
            capabilities = result["capabilities"]
        else:
            result = {"protocolVersion": 1, "agentCapabilities": {
                "mcpCapabilities": {"http": True}, "sessionCapabilities": {"resume": {}, "options": {}}}}
            capabilities = result["agentCapabilities"]
        if supported:
            capabilities["providers"] = {}
    elif method == "providers/list":
        assert supported and params == {}
        listed = True
        projected_main = main.copy()
        if configured:
            projected_main["baseUrl"] += "?query=fixture-private-value"
            projected_main["headers"] = {"Authorization": "Bearer fixture-private-value"}
        result = {"providers": [
            {"providerId": "main", "supported": ["anthropic", "_custom_provider"],
             "required": True, "current": projected_main},
            {"providerId": "optional", "supported": ["openai"], "required": False, "current": optional},
        ]}
    elif method == "providers/set":
        assert supported and listed and not configured
        assert params == {
            "providerId": "main", "apiType": "_custom_provider", "baseUrl": "https://configured.example.invalid/llm",
            "headers": {"authorization": "Bearer fixture-private-value", "x-project": "routing-fixture"},
            "_meta": {"test": {"mode": "full"}},
        }
        main = {key: params[key] for key in ("apiType", "baseUrl")}
        configured = True
        result = {}
    elif method == "providers/disable":
        assert supported and configured and params == {"providerId": "optional"}
        optional = None
        result = {}
    elif method in ("session/options", "session/new", "session/resume"):
        assert configured and optional is None
        assert Path(params["cwd"]) == Path.cwd()
        result = {}
        if method == "session/new":
            result = {"sessionId": session}
        elif method == "session/resume":
            assert params["sessionId"] == session
            assert Path(".providers-started").is_file()
    elif method == "session/prompt":
        assert params["sessionId"] == session and configured and optional is None
        turn = {"first routing prompt": "first", "continue under configured route": "second"}[params["prompt"][0]["text"]]
        Path(".providers-started").touch()
        content = {"type": "text", "text": "provider routing applied"}
        usage = {"inputTokens": 13, "outputTokens": 7, "cachedReadTokens": 5, "totalTokens": 26}
        if version == 2:
            send(id=identifier, result={"messageId": "prompt:" + turn})
            update("state_update", state="running")
            update("agent_message", messageId="reply:" + turn, content=[content])
            update("state_update", state="idle", stopReason="end_turn", usage=usage)
            continue
        update("agent_message_chunk", content=content)
        result = {"stopReason": "end_turn", "usage": usage}
    elif method == "session/close":
        result = {}
    else:
        raise AssertionError(method)
    send(id=identifier, result=result)
