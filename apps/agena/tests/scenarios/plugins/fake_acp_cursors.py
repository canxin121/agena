#!/usr/bin/env python3
import json
from pathlib import Path
import sys


cwd = Path.cwd()
version = int(Path(".cursor-version").read_text())
cut = Path(".cursor-rewound")
session = "native-cursors"


def send(**value):
    print(json.dumps({"jsonrpc": "2.0", **value}), flush=True)


def update(kind, **fields):
    send(method="session/update", params={
        "sessionId": session, "update": {"sessionUpdate": kind, **fields},
    })


def message(role, identifier, text):
    kind = f"{role}_message" if version == 2 else f"{role}_message_chunk"
    block = {"type": "text", "text": text}
    update(kind, messageId=identifier, content=[block] if version == 2 else block)


def grant(cursor, kind, message_id, position, **capabilities):
    update("cursor_update", cursorId=cursor, capabilities=capabilities, anchor={
        "type": kind, "messageId": message_id, "position": position,
    })


for line in sys.stdin:
    request = json.loads(line)
    if "id" not in request or "method" not in request:
        continue
    method, params = request["method"], request["params"]
    result = {}
    if method == "initialize":
        if version == 1 and params["protocolVersion"] == 2:
            send(id=request["id"], result={"protocolVersion": 1})
            break
        assert params["protocolVersion"] == version
        result = {"protocolVersion": version}
        if version == 1:
            assert params["clientCapabilities"]["session"]["cursors"] == {}
            air = params["clientCapabilities"]["_meta"]["jetbrains"]["air"]
            assert "sessionRewind" in air["capabilities"]
            result["agentCapabilities"] = {
                "loadSession": True, "sessionCapabilities": {"list": {}, "resume": {}, "fork": {}},
            }
            result["_meta"] = {"jetbrains": {"air": {"version": 1, "capabilities": ["sessionRewind"]}}}
        else:
            result["capabilities"] = {"session": {"fork": {}}}
    elif method == "session/list":
        result = {"sessions": [{"sessionId": session, "cwd": str(cwd), "title": "A cursor fixture"}]}
    elif method == "session/load" or method == "session/resume" and "replayFrom" in params:
        assert params["sessionId"] == session
        message("user", "first-user", "First saved prompt")
        message("agent", "first-agent", "First saved answer")
        if not cut.is_file():
            message("user", "second-user", "Second saved prompt")
            if version == 1:
                grant("opaque-cut", "agent_message", "first-agent", "after", fork={}, rewind={})
            else:
                grant("revoked-in-replay", "user_message", "second-user", "before", fork={})
                update("cursor_update", cursorId="revoked-in-replay", capabilities=None)
            message("agent", "second-agent", "Second saved answer")
        if version == 2:
            update("replay_complete")
    elif method == "session/resume":
        assert params["sessionId"] == session
    elif method == "session/fork":
        assert version == 1 and params["sessionId"] == session and params["cursorId"] == "opaque-cut"
        result = {"sessionId": "native-fork"}
    elif method == "_session/rewind":
        assert version == 1 and params == {"sessionId": session, "cursorId": "opaque-cut"}
        cut.touch()
        result = {"rewound": True}
    elif method == "session/prompt":
        assert params["sessionId"] == session
        prompt = params["prompt"][0]["text"]
        if version == 1:
            assert cut.is_file()
            message("agent", "after-rewind", "Continued original native conversation")
            result = {"stopReason": "end_turn"}
        else:
            send(id=request["id"], result={"messageId": prompt})
            update("state_update", state="running")
            if prompt == "Second live prompt":
                grant("late-live-cursor", "user_message", "First live prompt", "before", fork={})
            elif prompt == "Third live prompt":
                update("cursor_update", cursorId="late-live-cursor", capabilities={})
            message("agent", f"reply-{prompt}", "Prompt completed")
            update("state_update", state="idle", stopReason="end_turn")
            continue
    elif method != "session/close":
        raise AssertionError(method)
    send(id=request["id"], result=result)
