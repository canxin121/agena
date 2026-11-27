#!/usr/bin/env python3
"""Validate prompt context, initial option order, and inline native Skills."""

import json
from pathlib import Path
import sys


NATIVE = "startup-native"
FORK = "startup-fork"
PROMPT_RECORD = Path(".acp-startup-prompts.jsonl")
CONFIG_RECORD = Path(".acp-startup-options.jsonl")
VERSION = int(Path(".acp-startup-version").read_text())
NATIVE_SKILLS = ["summarize", "chart"]
EXPECTED_CONFIG = [
    {"configId": "fast-native", "type": "boolean", "value": True},
    {"configId": "native-model", "type": "id", "value": "second"},
]


def send(**message):
    print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


def reply(request, **result):
    send(id=request["id"], result=result)


def update(session, **body):
    send(method="session/update", params={"sessionId": session, "update": body})


def record(path, value):
    with path.open("a", encoding="utf-8") as file:
        file.write(json.dumps(value) + "\n")


def options(version, fast=False, model="first"):
    id_key = "configId" if version == 2 else "id"
    return [
        {id_key: "fast-native", "type": "boolean", "name": "Fast", "currentValue": fast},
        {id_key: "native-model", "type": "select", "name": "Model", "category": "model",
         "currentValue": model, "options": [{"value": "first", "name": "First"},
                                             {"value": "second", "name": "Second"}]},
    ]


def commands():
    return [{"type": "skill", "name": name, "description": name.title()} for name in NATIVE_SKILLS] + [
        {"type": "command", "name": "status", "description": "Status"},
    ]


def check_workspace(params):
    assert params["additionalDirectories"] == [str(Path.cwd() / "shared")], params
    options = params.get("_meta", {}).get("claudeCode", {}).get("options", {})
    has_agena = any(server.get("name") == "agena" for server in params.get("mcpServers", []))
    assert options == ({"allowedTools": ["mcp__agena__*"]} if has_agena else {}), options


for line in sys.stdin:
    request = json.loads(line)
    method, params = request["method"], request["params"]
    if method == "initialize":
        if VERSION == 1 and params["protocolVersion"] == 2:
            reply(request, protocolVersion=1)
            continue
        version = params["protocolVersion"]
        assert version == VERSION, params
        capability = {"options": {}, "additionalDirectories": {}, "systemPrompt": {}, "fork": {}, "close": {}}
        if version == 1:
            reply(request, protocolVersion=1, agentCapabilities={
                "sessionCapabilities": capability, "loadSession": True, "mcpCapabilities": {"http": True},
            })
        else:
            assert params["capabilities"]["session"]["commands"]["skill"] == {}, params
            capability.update(prompt={"skill": {"inline": {}}}, mcp={"http": {}})
            reply(request, protocolVersion=2, capabilities={"session": capability}, info={"name": "Startup fixture"})
    elif method == "session/options":
        check_workspace(params)
        assert "mcpServers" not in params and "systemPrompt" not in params, params
        assert params.get("sessionId") in (None, NATIVE), params
        record(CONFIG_RECORD, {"method": method, **params})
        preview = [{"type": "command", "name": "preview-only", "description": "Startup catalog"}, commands()[0]]
        reply(request, configOptions=options(version), **({"availableCommands": preview} if version == 2 else {}))
    elif method in ("session/new", "session/load", "session/resume", "session/fork"):
        check_workspace(params)
        current = FORK if method == "session/fork" else NATIVE
        real = method != "session/new" or bool(params["mcpServers"])
        fast, model = False, "first"
        if real:
            assert params["configOptions"] == EXPECTED_CONFIG, params
            assert params.get("sessionId") == (None if method == "session/new" else NATIVE), params
            assert params["systemPrompt"]["mode"] == "append", params
            assert len(params["systemPrompt"]["content"]) == 1, params
            instruction = params["systemPrompt"]["content"][0]["text"]
            assert "ACP_PROJECT_GUIDANCE_FIXTURE" in instruction, instruction
            assert "ACP_PROFILE_SKILL_FIXTURE" in instruction, instruction
            assert "Prefer the `agena` MCP server's tools" in instruction, instruction
            assert "Use native tools when the server is unavailable or has no suitable tool." in instruction
            record(PROMPT_RECORD, {"method": method, "text": instruction})
            # Echo applied values. The first setup leaves one otherwise
            # valid initial option unchanged so the client exercises its setter.
            fast = method != "session/new"
            model = "second"
        result = {"configOptions": options(version, fast, model)}
        if version == 2 and real:
            result["availableCommands"] = commands()
        if method in ("session/new", "session/fork"):
            result["sessionId"] = current
        reply(request, **result)
    elif method == "session/set_config_option":
        if params["configId"] == "fast-native":
            assert params["type"] == "boolean" and params["value"] is True, params
            fast = params["value"]
        else:
            assert params["configId"] == "native-model" and params["value"] == "second", params
            if version == 2:
                assert params["type"] == "id", params
            model = params["value"]
        record(CONFIG_RECORD, {"method": method, "model": model, "fast": fast})
        reply(request, configOptions=options(version, fast, model))
    elif method == "session/prompt":
        assert fast is True and model == "second", params
        assert current == NATIVE, params
        blocks = params["prompt"]
        if blocks[0].get("text") == "before ":
            assert version == 2 and blocks == [
                {"type": "text", "text": "before "},
                {"type": "skill", "name": "summarize"},
                {"type": "text", "text": " between "},
                {"type": "skill", "name": "chart"},
                {"type": "text", "text": " after"},
            ], blocks
        else:
            assert blocks == [{"type": "text", "text": "/status now"}], blocks
        content = {"type": "text", "text": "Startup configuration applied"}
        if version == 2:
            reply(request, messageId=f"user-{request['id']}")
            update(current, sessionUpdate="state_update", state="running")
            update(current, sessionUpdate="agent_message", messageId=f"reply-{request['id']}", content=[content])
            update(current, sessionUpdate="state_update", state="idle", stopReason="end_turn")
        else:
            update(current, sessionUpdate="agent_message_chunk", content=content)
            reply(request, stopReason="end_turn")
    elif method == "session/close":
        reply(request)
    else:
        raise AssertionError(request)
