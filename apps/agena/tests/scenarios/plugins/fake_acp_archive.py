#!/usr/bin/env python3
"""Standard ACP archive capability, paginated workspace listing, and persistence."""
import json
from pathlib import Path
import sys


directory = Path.cwd()
version = int(Path(".archive-version").read_text())
split_history = Path(".split-antigravity-history").is_file()
supports_resume = Path(".antigravity-supports-resume").is_file()
previous = "--app-data-dir" in sys.argv
state_file = Path(".archive-state")
sessions = json.loads(state_file.read_text()) if state_file.is_file() else {
    "saved": {"archived": True, "active": False},
    "ready": {"archived": False, "active": True},
}
if split_history and not previous:
    state_file = Path(".archive-managed-state")
    sessions = json.loads(state_file.read_text()) if state_file.is_file() else {}


def send(**message):
    print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


def log(method, detail=""):
    with Path(".archive-wire").open("a") as output:
        output.write(f"{method} {detail}\n")


def check_profile(profile):
    hooks = json.loads((profile / "config/hooks.json").read_text())
    assert set(hooks) == {"workspace-check"}, hooks
    assert hooks["workspace-check"] == {"PreToolUse": []}, hooks


if split_history:
    assert "--agent" not in sys.argv and "--include-default-customizations=false" not in sys.argv

if "--print" in sys.argv:
    assert split_history and sys.argv[sys.argv.index("--resume") + 1] == "ready"
    assert sys.argv[sys.argv.index("--print") + 1] == "/compact"
    assert (Path.home() / ".gemini/antigravity-cli/conversations/fixture").is_file()
    check_profile(Path.home() / ".gemini")
    log("compact-cli", "ready")
    print(json.dumps({"event": "init", "conversation_id": "ready"}))
    print(json.dumps({"event": "step_update", "step_update": {
        "conversation_id": "ready", "step_type": "checkpoint", "state": "DONE", "step_index": 9,
    }}))
    print(json.dumps({"event": "result", "result": {"conversation_id": "ready", "status": "SUCCESS"}}))
    sys.exit(0)


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
            if split_history:
                if not previous or supports_resume:
                    capabilities["resume"] = {}
                result["agentCapabilities"]["_meta"] = {
                    "antigravity": {"systemPrompt": {"append": True}},
                }
                profile = Path(sys.argv[sys.argv.index("--gemini-dir") + 1])
                check_profile(profile)
        else:
            result = {"protocolVersion": 2, "capabilities": {"session": capabilities}}
    elif method == "session/list":
        assert params["cwd"] == str(directory) and "_meta" not in params
        filter_state = params["archived"]
        if split_history and not previous and not Path(".managed-signed-in").is_file():
            send(id=request["id"], error={"code": -32000, "message": "Authentication required"})
            continue
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
    elif method in {"session/new", "session/resume"} and split_history and not previous:
        if method == "session/resume" and native_id not in sessions:
            send(id=request["id"], error={"code": -32002, "message": "Session not found"})
            continue
        meta = params["_meta"]["antigravity"]
        assert meta["eagerMcpServers"] == ["agena"]
        instruction = meta["systemPrompt"]["append"]
        assert "Prefer the `agena` MCP server's tools" in instruction, instruction
        log(f"{method}-system-prompt", instruction)
        if method == "session/new":
            sessions["managed-run"] = {"archived": False, "active": True}
            state_file.write_text(json.dumps(sessions))
            result = {"sessionId": "managed-run"}
        else:
            log(method, native_id)
            result = {}
    elif method in {"session/load", "session/resume"} and split_history and previous and params["mcpServers"]:
        assert native_id == "ready" and params["mcpServers"][0]["name"] == "agena"
        assert method == ("session/resume" if supports_resume else "session/load"), params
        instruction = params["_meta"]["antigravity"]["systemPrompt"]["append"]
        assert "Prefer the `agena` MCP server's tools" in instruction, instruction
        log("legacy-continuation", native_id)
        send(method="session/update", params={"sessionId": native_id, "update": {
            "sessionUpdate": "available_commands_update",
            "availableCommands": [{"name": "compact", "description": "Compact with optional summary guidance"}],
        }})
        result = {}
    elif method in {"session/load", "session/resume"}:
        if split_history:
            log("history-port", "previous" if previous else "managed")
            if native_id not in sessions:
                send(id=request["id"], error={"code": -32002, "message": "Session not found"})
                continue
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
    elif method == "session/prompt" and split_history:
        assert (not previous and native_id == "managed-run") or (
            previous and native_id == "ready"
        )
        log(method, native_id)
        if previous:
            text = params["prompt"][0]["text"]
            assert not text.startswith("/compact"), text
            if text == "Continue after native compaction":
                log("compact-followup", native_id)
        send(method="session/update", params={"sessionId": native_id, "update": {
            "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Native Antigravity"},
        }})
        result = {"stopReason": "end_turn"}
    elif method == "session/close":
        result = {}
    else:
        raise AssertionError(f"Unexpected ACP method {method}")
    send(id=request["id"], result=result)
