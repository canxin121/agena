#!/usr/bin/env python3
"""Standard ACP archive capability, paginated workspace listing, and persistence."""
import json
from pathlib import Path
import subprocess
import sys


directory = Path.cwd()
version = int(Path(".archive-version").read_text())
split_history = Path(".split-antigravity-history").is_file()
guarded_history = Path(".guarded-antigravity-history").is_file()
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


if "--print" in sys.argv:
    assert guarded_history and sys.argv[sys.argv.index("--resume") + 1] == "ready"
    assert sys.argv[sys.argv.index("--print") + 1] == "/compact"
    assert (Path.home() / ".gemini/antigravity-cli/conversations/fixture").is_file()
    assert (Path.home() / ".gemini/config/hooks.json").is_file()
    log("compact-cli", "ready")
    print(json.dumps({"event": "init", "conversation_id": "ready"}))
    print(json.dumps({"event": "step_update", "step_update": {
        "conversation_id": "ready", "step_type": "checkpoint", "state": "DONE", "step_index": 9,
    }}))
    print(json.dumps({"event": "result", "result": {"conversation_id": "ready", "status": "SUCCESS"}}))
    sys.exit(0)


def check_guard():
    profile = Path(sys.argv[sys.argv.index("--gemini-dir") + 1])
    hooks = json.loads((profile / "config/hooks.json").read_text())
    [gate] = hooks["agena-acp-tools"]["PreToolUse"]
    assert gate["matcher"] == "*"
    [handler] = gate["hooks"]
    for name, expected in (("write_file", "deny"), ("run_command", "deny"), ("search_web", "deny"),
                           ("mcp_other_tool", "deny"), ("mcp_agena_tool", "allow")):
        checked = subprocess.run(handler["command"], shell=True, capture_output=True, text=True,
                                 input=json.dumps({"toolCall": {"name": name}}), timeout=10, check=True)
        assert json.loads(checked.stdout)["decision"] == expected


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
            if split_history and (not previous or guarded_history):
                capabilities["resume"] = {}
                result["agentCapabilities"]["_meta"] = {
                    "antigravity": {"systemPrompt": {"append": True}},
                }
            if guarded_history:
                result["agentInfo"] = {"name": "Antigravity ACP Server", "version": "1.5.18"}
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
        log(f"{method}-system-prompt", meta["systemPrompt"]["append"])
        if method == "session/new":
            sessions["managed-run"] = {"archived": False, "active": True}
            state_file.write_text(json.dumps(sessions))
            result = {"sessionId": "managed-run"}
        else:
            log(method, native_id)
            result = {}
    elif method == "session/resume" and guarded_history and previous:
        assert native_id == "ready" and params["mcpServers"][0]["name"] == "agena"
        check_guard()
        log("legacy-resume", native_id)
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
            guarded_history and previous and native_id == "ready"
        )
        log(method, native_id)
        if guarded_history and previous:
            text = params["prompt"][0]["text"]
            assert not text.startswith("/compact"), text
            if text == "Continue after native compaction":
                log("compact-followup", native_id)
        send(method="session/update", params={"sessionId": native_id, "update": {
            "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Restricted Antigravity"},
        }})
        result = {"stopReason": "end_turn"}
    elif method == "session/close":
        result = {}
    else:
        raise AssertionError(f"Unexpected ACP method {method}")
    send(id=request["id"], result=result)
