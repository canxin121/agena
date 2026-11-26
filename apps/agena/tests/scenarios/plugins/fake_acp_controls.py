#!/usr/bin/env python3
"""ACP peer for configuration, permission, account, and targeted work controls."""

from pathlib import Path
import json
import sys
import time


legacy = Path(sys.argv[0]).name == "agy"
auth = Path(".acp-control-authenticated")
session = "control-session"
mode = "default"
fast = False
version = 1 if legacy else int(Path(".acp-controls-version").read_text())


def receive():
    line = sys.stdin.readline()
    if not line:
        raise EOFError
    return json.loads(line)


def send(**fields):
    print(json.dumps({"jsonrpc": "2.0", **fields}), flush=True)


def response(request, **payload):
    send(id=request["id"], result=payload)


def update(payload, source=session):
    send(method="session/update", params={"sessionId": source, "update": payload})


def options():
    choices = [{"id": "default", "name": "Default"}, {"id": "plan", "name": "Plan"}, {"id": "review", "name": "Review"}]
    if legacy:
        return {"modes": {"currentModeId": mode, "availableModes": choices}}
    advertised = [
        {"id": "native-mode", "type": "select", "category": "mode", "name": "Mode",
         "currentValue": mode, "options": [{"value": item["id"], "name": item["name"]} for item in choices]},
        {"id": "fast", "type": "boolean", "category": "model_config", "name": "Fast execution",
         "currentValue": fast}
    ]
    if version == 2:
        for option in advertised:
            option["configId"] = option.pop("id")
    return {"configOptions": advertised}


def request_url(identifier, address):
    send(id=identifier, method="elicitation/create", params={
        "sessionId": session, "mode": "url", "message": "Authorize a test tool",
        "url": address, "elicitationId": identifier,
    })
    reply = receive()
    assert reply["id"] == identifier, reply
    return reply


def wait_for_file(name):
    deadline = time.monotonic() + 20
    while not Path(name).is_file():
        assert time.monotonic() < deadline, name
        time.sleep(0.01)


def cancellation_flow(prompt):
    cap = {"cancel": {}}
    child = "work-child"
    response(prompt, messageId="work-message")
    update({"sessionUpdate": "state_update", "state": "running", "background": []})
    update({"sessionUpdate": "subagent_update", "sessionId": child, "title": "Busy worker",
            "state": {"state": "running"}, "capabilities": cap})
    update({"sessionUpdate": "subagent_update", "sessionId": "already-idle",
            "state": {"state": "idle"}, "capabilities": cap})
    update({"sessionUpdate": "tool_call", "toolCallId": "job", "title": "Worker job", "status": "in_progress"}, child)
    update({"sessionUpdate": "tool_call_update", "toolCallId": "job", "capabilities": cap}, child)
    update({"sessionUpdate": "tool_call_update", "toolCallId": "job", "title": "Uncancellable root job", "status": "in_progress"})
    update({"sessionUpdate": "tool_call_update", "toolCallId": "completed", "status": "completed", "capabilities": cap})
    update({"sessionUpdate": "terminal_update", "terminalId": "foreign", "capabilities": cap}, "unannounced")
    update({"sessionUpdate": "terminal_update", "terminalId": "term", "command": "Active terminal", "capabilities": cap})
    update({"sessionUpdate": "terminal_update", "terminalId": "revoked", "capabilities": cap})
    update({"sessionUpdate": "terminal_update", "terminalId": "bad-shape", "capabilities": {"cancel": True}})
    update({"sessionUpdate": "state_update", "state": "idle", "stopReason": "end_turn", "background": [
        {"type": "terminal", "terminalId": "term"}, {"type": "terminal", "terminalId": "revoked"},
        {"type": "subagent", "sessionId": child},
    ]})
    update({"sessionUpdate": "agent_message", "messageId": "work-ready",
            "content": [{"type": "text", "text": "Targets are visible"}]})

    wait_for_file(".acp-work-revoke")
    update({"sessionUpdate": "terminal_update", "terminalId": "revoked", "capabilities": None})

    request = receive()
    assert request["method"] == "terminal/cancel", request
    assert request["params"] == {"sessionId": session, "terminalId": "term"}, request
    response(request)
    update({"sessionUpdate": "terminal_update", "terminalId": "term", "exitStatus": {"signal": "SIGTERM"}})

    for should_retry in (True, False):
        request = receive()
        assert request["method"] == "tool_call/cancel", request
        assert request["params"] == {"sessionId": child, "toolCallId": "job"}, request
        if should_retry:
            send(id=request["id"], error={"code": -32603, "message": "Temporary failure"})
        else:
            response(request)
    update({"sessionUpdate": "tool_call_update", "toolCallId": "job", "status": "cancelled"}, child)

    notice = receive()
    assert notice["method"] == "session/cancel" and "id" not in notice, notice
    assert notice["params"] == {"sessionId": child}, notice
    update({"sessionUpdate": "subagent_update", "sessionId": child,
            "state": {"state": "idle", "stopReason": "cancelled"}})
    update({"sessionUpdate": "tool_call_update", "toolCallId": "job", "status": "completed"})
    update({"sessionUpdate": "agent_message", "messageId": "work-done",
            "content": [{"type": "text", "text": "Parent kept going"}]})
    update({"sessionUpdate": "state_update", "state": "idle", "background": []})

    wait_for_file(".acp-work-idle")
    update({"sessionUpdate": "terminal_update", "terminalId": "late", "capabilities": cap})
    update({"sessionUpdate": "state_update", "state": "idle", "background": [{"type": "terminal", "terminalId": "late"}]})
    request = receive()
    assert request["method"] == "terminal/cancel", request
    assert request["params"] == {"sessionId": session, "terminalId": "late"}, request
    response(request)
    update({"sessionUpdate": "terminal_update", "terminalId": "late", "exitStatus": {"signal": "SIGTERM"}})


try:
    while True:
        request = receive()
        params = request.get("params", {})
        method = request["method"]
        if method == "initialize":
            if version == 1 and params["protocolVersion"] == 2:
                response(request, protocolVersion=1)
                break
            assert params["protocolVersion"] == version
            caps = params.get("capabilities" if version == 2 else "clientCapabilities", {})
            methods = [{"id": "browser", "name": "Sign in with browser"}]
            if "terminal" in caps.get("auth", {}):
                methods += [
                    {"id": "terminal", "name": "Terminal login", "type": "terminal", "args": ["--login"],
                     "env": {"NO_BROWSER": "1"}},
                    {"id": "unsafe-terminal", "name": "Unsafe launch", "type": "terminal", "args": ["--login"],
                     "env": {"PATH": "/unsafe"}},
                ]
            assert caps["elicitation"]["url"] == {}, caps
            if version == 1:
                assert caps["session"]["configOptions"]["boolean"] == {}, caps
                response(request, protocolVersion=1, authMethods=methods, agentCapabilities={
                    "mcpCapabilities": {"http": True},
                    "auth": {"logout": {}, "status": {}},
                    "sessionCapabilities": {"close": {}, "delete": {}, "resume": {}},
                })
            else:
                for entry in methods:
                    entry["methodId"] = entry.pop("id")
                    if "env" in entry:
                        entry["env"] = [{"name": name, "value": value} for name, value in entry["env"].items()]
                    entry.setdefault("type", "agent")
                response(request, protocolVersion=2, authMethods=methods, info={"name": "controls"},
                         capabilities={"session": {"delete": {}, "mcp": {"http": {}}}, "auth": {"status": {}}})
        elif method in ("authenticate", "auth/login"):
            assert method == ("auth/login" if version == 2 else "authenticate"), method
            assert params == {"methodId": "browser"}, params
            auth.touch()
            response(request)
        elif method == "auth/status":
            if auth.is_file():
                response(request, state="authenticated", accounts=[{"label": "Test account", "active": True}])
            else:
                response(request, state="unauthenticated")
        elif method in ("logout", "auth/logout"):
            assert method == ("auth/logout" if version == 2 else "logout"), method
            auth.unlink(missing_ok=True)
            response(request)
        elif method == "session/new":
            response(request, sessionId=session, **options())
        elif method in ("session/resume", "session/load"):
            response(request, **options())
        elif method in ("session/close", "session/delete"):
            response(request)
        elif method == "session/set_mode":
            assert legacy and params["modeId"] == "plan", params
            mode = params["modeId"]
            response(request)
        elif method == "session/set_config_option":
            assert not legacy, params
            if params["configId"] == "fast":
                assert isinstance(params["value"], bool), params
                assert params["type"] == "boolean", params
                fast = params["value"]
            else:
                assert params["configId"] == "native-mode", params
                if version == 2:
                    assert params["type"] == "id", params
                mode = params["value"]
            response(request, **options())
        elif method == "session/prompt":
            if version == 2 and params["prompt"][0].get("text") == "check targeted cancellation":
                cancellation_flow(request)
                continue
            assert mode == "plan", mode
            if legacy:
                mode = "review"
                update({"sessionUpdate": "current_mode_update", "currentModeId": mode})
                text = "legacy mode succeeded"
            else:
                assert fast
                mode = "review"
                update({"sessionUpdate": "config_option_update", **options()})
                send(id="edit", method="session/request_permission", params={
                    "sessionId": session,
                    "toolCall": {"toolCallId": "edit-tool", "title": "Edit a project file", "kind": "edit"},
                    "options": [
                        {"optionId": "yes1", "name": "Approve edit", "kind": "allow_once"},
                        {"optionId": "yes_all", "name": "Remember approval", "kind": "allow_always"},
                        {"optionId": "no1", "name": "Deny edit", "kind": "reject_once"},
                        {"optionId": "no_all", "name": "Remember denial", "kind": "reject_always"},
                    ],
                })
                assert receive()["result"] == {"outcome": {"outcome": "selected", "optionId": "yes_all"}}
                denied = request_url("unsafe-link", "javascript:alert(1)")
                assert denied["error"]["code"] == -32602, denied
                accepted = request_url("test-link", "https://example.invalid/connect?state=test")
                assert accepted["result"] == {"action": "accept"}, accepted
                send(method="elicitation/complete", params={"elicitationId": "bogus"})
                send(method="elicitation/complete", params={"elicitationId": "test-link"})
                text = "native controls succeeded"
            update({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}})
            response(request, stopReason="end_turn")
        else:
            raise AssertionError(f"Unexpected method {method!r}")
except EOFError:
    pass
