#!/usr/bin/env python3
"""ACP account notices and the negotiated v1 Claude background-task stop."""

import json
from pathlib import Path
import sys
import time


SESSION = "auth-work-root"
CHILD = "auth-work-child"
CLAUDE = Path(sys.argv[0]).name == "claude-agent-acp"


def send(**fields):
    print(json.dumps({"jsonrpc": "2.0", **fields}), flush=True)


def reply(request, **result):
    send(id=request["id"], result=result)


def update(kind, source=SESSION, **fields):
    send(method="session/update", params={"sessionId": source, "update": {"sessionUpdate": kind, **fields}})


def auth(state, **fields):
    send(method="auth/status_update", params={"state": state, **fields})


def wait_for_marker(name):
    deadline = time.monotonic() + 20
    while not Path(name).is_file():
        assert time.monotonic() < deadline, name
        time.sleep(0.01)


def spawn_task(task, source=SESSION, stop=True):
    update("async_task_spawned", source, asyncTaskId=task, name=f"Work {task}", taskType="shell",
           description="Fixture background task", showInTranscript=True, canStop=stop)


version = int(Path(".auth-work-version").read_text()) if CLAUDE else 1
air = False
for line in sys.stdin:
    request = json.loads(line)
    method, params = request["method"], request["params"]
    if method == "initialize":
        if version == 1 and params["protocolVersion"] == 2:
            reply(request, protocolVersion=1)
            assert sys.stdin.readline() == "", "Downgrade requires a fresh connection"
            Path(f".{Path(sys.argv[0]).name}-downgraded").touch()
            break
        assert params["protocolVersion"] == version
        capabilities = params["capabilities" if version == 2 else "clientCapabilities"]
        terminal = capabilities.get("auth", {}).get("terminal")
        methods = []
        if terminal == ({} if version == 2 else True):
            method_key = "methodId" if version == 2 else "id"
            methods = [{method_key: "terminal", "type": "terminal", "name": "Terminal", "args": ["--login"]}]
        if version == 1:
            air_meta = capabilities.get("_meta", {}).get("jetbrains", {}).get("air")
            air = air_meta is not None and "asyncTasks" in air_meta["capabilities"]
            if air:
                assert CLAUDE and air_meta["version"] == 1, params
                assert set(air_meta["capabilities"]) == {"asyncTasks", "sessionRewind"}, params
                assert terminal is None, params
            agent = {"mcpCapabilities": {"http": True}, "sessionCapabilities": {"close": {}}}
            if CLAUDE:
                agent["auth"] = {"status": {}, "logout": {}}
            extra = {"_meta": {"jetbrains": {"air": air_meta}}} if air else {}
            reply(request, protocolVersion=1, authMethods=methods, agentCapabilities=agent, **extra)
        else:
            assert CLAUDE and "air" not in capabilities.get("_meta", {}).get("jetbrains", {}), params
            reply(request, protocolVersion=2, authMethods=methods, info={"name": "Fixture"},
                  capabilities={"session": {"mcp": {"http": {}}}, "auth": {"status": {}}})
    elif method in ("auth/status", "auth/logout", "logout"):
        assert params == {} and not air, request
        if method != "auth/status":
            assert methods and method == ("auth/logout" if version == 2 else "logout"), request
            reply(request)
        else:
            reply(request, state="not_required")
    elif method == "session/new":
        reply(request, sessionId=SESSION)
    elif method == "session/prompt":
        text = params["prompt"][0]["text"]
        if version == 2:
            reply(request, messageId="auth-work-prompt")
            update("state_update", state="running")
        if text == "control legacy tasks":
            assert version == 1 and air, request
            update("subagent_spawned", subagentSessionId=CHILD, name="Helper", task="Run background work")
            spawn_task("child-task", CHILD)
            spawn_task("completed")
            spawn_task("refused")
            spawn_task("unsupported", stop=False)
            spawn_task("unannounced", "foreign")
            spawn_task(" child-task", CHILD)
            update("async_task_state_update", asyncTaskId="completed", state="completed")
            update("subagent_state_update", subagentSessionId=CHILD, state="completed")
        else:
            assert text == "report native account", request
            auth("authenticated", accounts=[{
                "label": "Live account", "active": True, "name": "Fixture", "description": "Native account",
                "email": "fixture@example.invalid", "organization": "Fixture org",
                "_meta": {"secret": "do-not-export"},
            }], _meta={"secret": "do-not-export"})
            if not CLAUDE:
                spawn_task("unadvertised")
        content = {"type": "text", "text": "Native work initialized"}
        if version == 1:
            update("agent_message_chunk", content=content)
            reply(request, stopReason="end_turn")
        else:
            update("agent_message", messageId="auth-work-reply", content=[content])
            update("state_update", state="idle", stopReason="end_turn")
        if text == "report native account" and CLAUDE:
            wait_for_marker(f".auth-work-update-{version}")
            auth("other_future_state")
    elif method == "_session/async_task/stop":
        assert version == 1 and air, request
        assert params["sessionId"] == SESSION, request
        target = params["asyncTaskId"]
        assert target in ("child-task", "refused"), request
        if target == "child-task":
            update("async_task_state_update", CHILD, asyncTaskId=target, state="stopped")
        reply(request, stopped=target == "child-task")
    elif method == "session/close":
        assert params == {"sessionId": SESSION}, request
        reply(request)
    else:
        raise AssertionError(method)
