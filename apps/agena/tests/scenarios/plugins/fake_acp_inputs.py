#!/usr/bin/env python3
"""Exercise request-scoped auth and independent input on a parked ACP peer."""

import json
from pathlib import Path
import sys
import time


SESSION = "input-session"


def write(value):
    print(json.dumps({"jsonrpc": "2.0", **value}), flush=True)


def read():
    return json.loads(sys.stdin.readline())


def update(**data):
    write({"method": "session/update", "params": {"sessionId": SESSION, "update": data}})


def ask(identifier, method, params):
    write({"id": identifier, "method": method, "params": params})
    frame = read()
    assert frame["id"] == identifier, frame
    return frame


def form(prompt, **scope):
    return {
        **scope, "mode": "form", "message": prompt,
        "requestedSchema": {
            "type": "object", "properties": {"count": {"type": "integer", "minimum": 1, "maximum": 10}},
            "required": ["count"],
        },
    }


def url(identifier):
    return {
        "sessionId": SESSION, "mode": "url", "message": identifier,
        "url": "https://example.invalid/connect", "elicitationId": identifier,
    }


def wait_for(marker):
    deadline = time.monotonic() + 20
    while not Path(marker).is_file():
        assert time.monotonic() < deadline, marker
        time.sleep(0.01)


authenticated = False
for line in sys.stdin:
    request = json.loads(line)
    method, params, identifier = request["method"], request["params"], request.get("id")
    if method == "initialize":
        assert params["protocolVersion"] == 2
        assert params["capabilities"]["elicitation"] == {"form": {}, "url": {}}
        result = {
            "protocolVersion": 2, "info": {"name": "ACP inputs fixture"},
            "capabilities": {"session": {"mcp": {"http": {}}}, "auth": {"status": {}}},
            "authMethods": [{"type": "agent", "methodId": "interactive", "name": "Interactive"}],
        }
    elif method == "auth/login":
        assert params["methodId"] == "interactive"
        wrong = ask("wrong-request", "elicitation/create", form("Wrong request", requestId=identifier + 1))
        assert wrong["result"] == {"action": "cancel"}, wrong
        answer = ask("auth-form", "elicitation/create", form("Confirm sign-in", requestId=identifier))
        assert answer["result"] == {"action": "accept", "content": {"count": 7}}, answer
        authenticated = True
        result = {}
    elif method == "auth/status":
        result = {"state": "authenticated" if authenticated else "unauthenticated"}
    elif method == "session/new":
        result = {"sessionId": SESSION}
    elif method == "session/resume":
        assert params["sessionId"] == SESSION
        result = {}
    elif method == "session/prompt":
        assert params["sessionId"] == SESSION
        prompt = params["prompt"][0]["text"]
        write({"id": identifier, "result": {"messageId": f"user:{identifier}"}})
        update(sessionUpdate="state_update", state="running")
        update(sessionUpdate="agent_message", messageId=f"assistant:{identifier}", content=[
            {"type": "text", "text": prompt + " complete"},
        ])
        update(sessionUpdate="state_update", state="idle", stopReason="end_turn")
        if prompt == "Start":
            wait_for(".ask-while-idle")
            wrong = ask("wrong-session", "elicitation/create", form("Wrong session", sessionId="other"))
            assert wrong["result"] == {"action": "cancel"}, wrong
            mcp = ask("late-tool", "mcp/message", {"sessionId": SESSION, "method": "tools/call"})
            assert mcp["error"]["code"] == -32602, mcp
            answered = ask("background-permission", "session/request_permission", {
                "sessionId": SESSION, "title": "Background permission",
                "subject": {"type": "tool_call", "toolCall": {"toolCallId": "native-tool"}},
                "options": [
                    {"optionId": "yes", "name": "Allow once", "kind": "allow_once"},
                    {"optionId": "no", "name": "Reject once", "kind": "reject_once"},
                ],
            })
            assert answered["result"] == {"outcome": {"outcome": "selected", "optionId": "no"}}, answered
            write({"id": "withdrawn-url", "method": "elicitation/create", "params": url("Withdrawn link")})
            wait_for(".withdraw-idle")
            write({"method": "$/cancel_request", "params": {"requestId": "withdrawn-url"}})
            response = read()
            assert response["id"] == "withdrawn-url" and response["result"] == {"action": "cancel"}, response
            approved = ask("open-url", "elicitation/create", url("Open link"))
            assert approved["result"] == {"action": "accept"}, approved
            write({"method": "elicitation/complete", "params": {"elicitationId": "Open link"}})
            write({"id": "old-form", "method": "elicitation/create", "params": form("Unanswered form", sessionId=SESSION)})
        continue
    elif method == "session/close":
        result = {}
    else:
        raise AssertionError(method)
    write({"id": identifier, "result": result})
