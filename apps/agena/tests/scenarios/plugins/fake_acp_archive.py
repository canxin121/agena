#!/usr/bin/env python3
"""Standard ACP archive capability, paginated workspace listing, and persistence."""
import json
from pathlib import Path
import sys


directory = Path.cwd()
version = int(Path(".archive-version").read_text())
antigravity_native = Path(".antigravity-native-history").is_file()
supports_resume = Path(".antigravity-supports-resume").is_file()
state_file = Path(".archive-state")
sessions = json.loads(state_file.read_text()) if state_file.is_file() else {
    "saved": {"archived": True, "active": False},
    "ready": {"archived": False, "active": True},
}


def send(**message):
    print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


def log(method, detail=""):
    with Path(".archive-wire").open("a") as output:
        output.write(f"{method} {detail}\n")


def check_native_store(store):
    hooks = json.loads((store / "config/hooks.json").read_text())
    assert set(hooks) == {"workspace-check"}, hooks
    assert hooks["workspace-check"] == {"PreToolUse": []}, hooks
    assert not (store / "agena-acp").exists()


if antigravity_native:
    assert sys.argv[1:] in (["acp", "--process-per-session"], ["acp"]), sys.argv


for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request:
        continue
    method, params = request["method"], request.get("params", {})
    native_id = params.get("sessionId")
    if method == "initialize":
        if version == 1 and params["protocolVersion"] == 2:
            send(id=request["id"], result={"protocolVersion": 1})
            break
        assert params["protocolVersion"] == version
        assert "_meta" not in params.get("clientCapabilities", {})
        capabilities = {"archive": {}, "delete": {}}
        if version == 1:
            capabilities.update(list={}, close={})
            result = {"protocolVersion": 1, "agentCapabilities": {
                "sessionCapabilities": capabilities, "loadSession": True,
            }}
            if antigravity_native:
                if supports_resume:
                    capabilities["resume"] = {}
                result["agentCapabilities"]["_meta"] = {
                    "antigravity": {"systemPrompt": {"append": True}},
                }
                check_native_store(Path.home() / ".gemini")
        else:
            result = {"protocolVersion": 2, "capabilities": {"session": capabilities}}
    elif method == "session/list":
        assert params["cwd"] == str(directory) and "_meta" not in params
        filter_state = params["archived"]
        assert filter_state in {"exclude", "include", "only"}
        expected_cursor = "" if version == 2 else f"second/{filter_state}"
        if "cursor" in params:
            assert params["cursor"] == expected_cursor
        rows = [
            {"sessionId": session, "cwd": str(directory), "title": session.title(),
             "archived": data["archived"]}
            for session, data in sessions.items()
            if filter_state == "include" or data["archived"] == (filter_state == "only")
        ]
        log(method, filter_state + (" next" if "cursor" in params else " first"))
        if "cursor" in params:
            result = {"sessions": rows[1:]}
        else:
            result = {"sessions": [
                {"sessionId": "foreign", "cwd": str(directory / "nested"), "archived": True},
                *rows[:1],
            ]}
            if len(rows) > 1:
                result["nextCursor"] = expected_cursor
    elif method in {"session/archive", "session/unarchive", "session/delete"}:
        assert params == {"sessionId": native_id} and native_id in sessions
        log(method, native_id)
        if method == "session/delete":
            del sessions[native_id]
        else:
            data = sessions[native_id]
            data["archived"] = method == "session/archive"
            if data["archived"] and data["active"]:
                log("closed-active", native_id)
                data["active"] = False
        state_file.write_text(json.dumps(sessions))
        result = {}
    elif method == "session/new" and antigravity_native:
        assert sys.argv[1:] == ["acp", "--process-per-session"], sys.argv
        meta = params["_meta"]["antigravity"]
        assert meta["eagerMcpServers"] == ["agena"]
        instruction = meta["systemPrompt"]["append"]
        assert "Prefer the `agena` MCP server's tools" in instruction, instruction
        log("session/new-system-prompt", instruction)
        sessions["managed-run"] = {"archived": False, "active": True}
        state_file.write_text(json.dumps(sessions))
        result = {"sessionId": "managed-run"}
    elif method in {"session/load", "session/resume"} and antigravity_native and params["mcpServers"]:
        assert sys.argv[1:] == ["acp", "--process-per-session"], sys.argv
        if native_id not in sessions:
            send(id=request["id"], error={"code": -32002, "message": "Session not found"})
            continue
        assert params["mcpServers"][0]["name"] == "agena"
        assert method == ("session/resume" if supports_resume else "session/load"), params
        meta = params["_meta"]["antigravity"]
        assert meta["eagerMcpServers"] == ["agena"]
        instruction = meta["systemPrompt"]["append"]
        assert "Prefer the `agena` MCP server's tools" in instruction, instruction
        if native_id == "ready":
            log("native-continuation", native_id)
            send(method="session/update", params={"sessionId": native_id, "update": {
                "sessionUpdate": "available_commands_update",
                "availableCommands": [{"name": "compact", "description": "Compact with optional summary guidance"}],
            }})
        else:
            log(f"{method}-system-prompt", instruction)
            log(method, native_id)
        result = {}
    elif method in {"session/load", "session/resume"}:
        assert native_id in sessions and params["cwd"] == str(directory)
        assert method == ("session/load" if version == 1 else "session/resume")
        log(method, native_id)
        send(method="session/update", params={"sessionId": native_id, "update": {
            "sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "Saved history"},
        }})
        if version == 2:
            assert params["replayFrom"] == {"type": "start"}
            send(method="session/update", params={"sessionId": native_id, "update": {"sessionUpdate": "replay_complete"}})
        result = {}
    elif method == "session/prompt" and antigravity_native:
        assert native_id in {"ready", "managed-run"}
        log(method, native_id)
        if native_id == "ready":
            text = params["prompt"][0]["text"]
            if text == "/compact Keep API context":
                log("compact-acp", native_id)
        send(method="session/update", params={"sessionId": native_id, "update": {
            "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Native Antigravity"},
        }})
        result = {"stopReason": "end_turn"}
    elif method == "session/close":
        result = {}
    else:
        raise AssertionError(f"Unexpected ACP method {method}")
    send(id=request["id"], result=result)
