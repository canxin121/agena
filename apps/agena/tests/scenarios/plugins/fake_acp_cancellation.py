#!/usr/bin/env python3
"""The ACP peer checks cancellation at the request and session boundaries."""

import json
from pathlib import Path
import sys
from threading import Event, Thread
from time import sleep


SESSION = "cancellation-session"
version = int(Path(".acp-cancellation-version").read_text())


def receive():
    line = sys.stdin.readline()
    if not line:
        raise EOFError
    return json.loads(line)


def send(**message):
    print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


def response(request, **result):
    send(id=request["id"], result=result)


def cancelled(request):
    send(id=request["id"], error={"code": -32800, "message": "Request cancelled"})


def update(**data):
    send(method="session/update", params={"sessionId": SESSION, "update": data})


def expect_request_cancel(request):
    frame = receive()
    assert frame == {
        "jsonrpc": "2.0", "method": "$/cancel_request", "params": {"requestId": request["id"]},
    }, frame


def permission(identifier, title):
    send(id=identifier, method="session/request_permission", params={
        "sessionId": SESSION,
        "title": title,
        "subject": {"type": "tool_call", "toolCall": {"toolCallId": str(identifier)}},
        "options": [{"optionId": "reject", "name": "Reject once", "kind": "reject_once"}],
    })


def begin(prompt):
    response(prompt, messageId="user-message")
    update(sessionUpdate="terminal_update", terminalId="surviving-terminal", capabilities={"cancel": {}})
    update(sessionUpdate="state_update", state="running", background=[{
        "type": "terminal", "terminalId": "surviving-terminal",
    }])


def scoped(prompt):
    begin(prompt)
    incoming = prompt["id"] + 1
    permission(incoming, "Independent permission")
    request = receive()
    assert request["method"] == "terminal/cancel" and request["id"] == incoming, request
    assert request["params"] == {"sessionId": SESSION, "terminalId": "surviving-terminal"}, request
    expect_request_cancel(request)
    cancelled(request)
    Path(".acp-target-timeout").touch()
    assert receive() == {"jsonrpc": "2.0", "id": incoming,
                         "result": {"outcome": {"outcome": "selected", "optionId": "reject"}}}
    update(sessionUpdate="terminal_update", terminalId="surviving-terminal", exitStatus={"exitCode": 0})
    update(sessionUpdate="agent_message", messageId="scoped-complete",
           content=[{"type": "text", "text": "Independent work continued"}])
    update(sessionUpdate="state_update", state="idle", stopReason="end_turn", background=[])


def stop_foreground(prompt):
    begin(prompt)
    permission("background", "Background permission")
    permission("foreground", "Foreground permission")
    request = receive()
    assert request == {"jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": SESSION}}, request
    send(method="$/cancel_request", params={"requestId": "foreground"})
    assert receive() == {"jsonrpc": "2.0", "id": "foreground", "result": {"outcome": {"outcome": "cancelled"}}}
    update(sessionUpdate="state_update", state="idle", stopReason="cancelled", background=[{
        "type": "terminal", "terminalId": "surviving-terminal",
    }])
    assert sys.stdin.readline() == "", "The client sent another cancellation after stopping the foreground"
    Path(".acp-foreground-stopped").touch()


def finish_silent_background(prompt):
    begin(prompt)
    update(sessionUpdate="state_update", state="idle", stopReason="end_turn",
           background=[{"type": "terminal", "terminalId": "surviving-terminal"}])
    # A live terminal can stay silent past the foreground inactivity deadline.
    sleep(2)
    update(sessionUpdate="terminal_update", terminalId="surviving-terminal", exitStatus={"exitCode": 0})
    update(sessionUpdate="state_update", state="idle", background=[])
    sleep(0.05)
    update(sessionUpdate="state_update", state="running", background=[])
    update(sessionUpdate="agent_message", messageId="background-followup",
           content=[{"type": "text", "text": "Background follow-up completed"}])
    update(sessionUpdate="state_update", state="idle", stopReason="end_turn", background=[])


def stall(prompt):
    if version == 2:
        response(prompt, messageId="inactive-user-message")
        update(sessionUpdate="state_update", state="running")
    else:
        update(sessionUpdate="agent_message_chunk", content={"type": "text", "text": "Partial response"})
    stopped = Event()

    def noise():
        while not stopped.wait(0.15):
            send(method="session/update", params={"sessionId": "different-session", "update": {
                "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "unrelated"},
            }})
            update(sessionUpdate="state_update", state="running")

    worker = Thread(target=noise, daemon=True)
    worker.start()
    try:
        frame = receive()
        assert frame == {
            "jsonrpc": "2.0", "method": "session/cancel", "params": {"sessionId": SESSION},
        }, frame
    finally:
        stopped.set()
        worker.join()
    Path(".acp-inactive-prompt-stopped").touch()
    if version == 2:
        update(sessionUpdate="state_update", state="idle", stopReason="cancelled")
    else:
        cancelled(prompt)


try:
    while True:
        request = receive()
        method = request["method"]
        params = request.get("params", {})
        if method == "initialize":
            if version == 1 and params["protocolVersion"] == 2:
                Path(".acp-rejected-v2-init").touch()
                send(id=request["id"], error={"code": -32602, "message": "Expected ACP v1 parameters"})
                assert sys.stdin.readline() == "", "The rejected connection must not be reused"
                break
            assert params["protocolVersion"] == version
            if version == 1:
                assert "clientCapabilities" in params
                response(request, protocolVersion=1, agentCapabilities={"mcpCapabilities": {"http": True}})
            else:
                response(request, protocolVersion=2, info={"name": "Cancellation fixture"},
                         capabilities={"session": {"mcp": {"http": {}}}})
        elif method == "session/new":
            if Path(".acp-hold-setup").exists():
                Path(".acp-setup-started").touch()
                expect_request_cancel(request)
                cancelled(request)
                Path(".acp-setup-cancelled").touch()
                assert sys.stdin.readline() == "", "Setup cancellation stopped a session that had not started"
                break
            response(request, sessionId=SESSION)
        elif method == "session/prompt":
            prompt = params["prompt"][0]["text"]
            if prompt == "timeout one native request":
                scoped(request)
            elif prompt == "wait for silent background":
                finish_silent_background(request)
            elif prompt == "stop sending response updates":
                stall(request)
                break
            else:
                assert prompt == "stop foreground", prompt
                stop_foreground(request)
                break
        else:
            raise AssertionError(request)
except EOFError:
    pass
