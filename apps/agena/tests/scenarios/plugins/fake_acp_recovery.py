#!/usr/bin/env python3
import json
from pathlib import Path
import sys


def reply(**message):
    print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


def update(session, kind, **fields):
    reply(method="session/update", params={"sessionId": session, "update": {"sessionUpdate": kind, **fields}})


version = int(Path(".recovery-version").read_text())
mode = Path(".recovery-mode").read_text()
record = Path(".recovery-record.json")
interrupted = Path(".recovery-interrupted")
created = Path(".recovery-created")

for line in sys.stdin:
    message = json.loads(line)
    if "id" not in message or "method" not in message:
        continue
    method, params = message["method"], message.get("params", {})
    ident, session = message["id"], params.get("sessionId")
    if method == "initialize":
        if version == 2:
            result = {"protocolVersion": 2, "capabilities": {"session": {"mcp": {"http": {}}}}}
        else:
            result = {"protocolVersion": 1, "agentCapabilities": {"loadSession": True, "mcpCapabilities": {"http": True}}}
    elif method == "session/new":
        assert not created.exists(), "retry detached the native conversation"
        created.touch()
        result = {"sessionId": "recovery-native"}
    elif method in ("session/load", "session/resume"):
        assert session == "recovery-native"
        if version == 2:
            if mode == "explicit-rejection":
                assert "replayFrom" not in params, "a rejected request did not insert a message"
            if params.get("replayFrom") == {"type": "start"}:
                if record.exists():
                    saved = json.loads(record.read_text())
                    update(session, "user_message", messageId=saved["id"], content=saved["content"])
                if mode != "markerless-replay":
                    update(session, "replay_complete")
            update(session, "state_update", state="idle")
        result = {}
    elif method == "session/prompt":
        assert session == "recovery-native"
        content = params["prompt"]
        prompt = content[0]["text"]
        key = params.get("idempotencyKey")
        with Path(".recovery-wire.jsonl").open("a") as wire:
            wire.write(json.dumps({"id": session, "input": prompt, "key": key}) + "\n")
        if prompt == "interrupted prompt" and not interrupted.exists():
            interrupted.touch()
            if mode == "explicit-rejection":
                code = -32000 if version == 1 else -32602
                reply(id=ident, error={"code": code, "message": "Fixture rejected before insertion"})
                continue
            if mode != "before-insertion":
                record.write_text(json.dumps({"id": "accepted-before-disconnect", "key": key, "content": content}))
            if version == 2 and mode == "after-response":
                update(session, "user_message", messageId="accepted-before-disconnect", content=content)
                reply(id=ident, result={"messageId": "accepted-before-disconnect"})
            break
        if version == 2 and prompt == "interrupted prompt" and record.exists():
            saved = json.loads(record.read_text())
            assert saved["key"] == key, "retry changed the idempotency key"
            reply(id=ident, result={"messageId": saved["id"]})
            continue
        if version == 2:
            message_id = "user-" + prompt
            update(session, "user_message", messageId=message_id, content=content)
            reply(id=ident, result={"messageId": message_id})
            update(session, "state_update", state="running")
        update(session, "agent_message_chunk", messageId="answer-" + prompt,
               content={"type": "text", "text": "Continued on the original conversation"})
        if version == 2:
            update(session, "state_update", state="idle", stopReason="end_turn")
            continue
        result = {"stopReason": "end_turn"}
    else:
        result = {}
    reply(id=ident, result=result)
