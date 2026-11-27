#!/usr/bin/env python3
"""Observe current-turn ACP input and Claude SDK options after the real hook chain."""

import json
from pathlib import Path
import sys
import traceback


NATIVE_ID = "acp-input-hooks"
TURN_COUNT = Path(".acp-input-hook-turns")
turns = int(TURN_COUNT.read_text()) if TURN_COUNT.is_file() else 0


def report_failure(*error):
    Path(".acp-input-hook-error").write_text("".join(traceback.format_exception(*error)))


sys.excepthook = report_failure


def send(**message):
    print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


for line in sys.stdin:
    request = json.loads(line)
    method, params = request["method"], request["params"]
    if "id" not in request:
        continue
    result = {}
    if method == "initialize":
        result = {"protocolVersion": 1, "agentCapabilities": {
            "loadSession": True, "mcpCapabilities": {"http": True},
            "sessionCapabilities": {"resume": {}, "systemPrompt": {}},
        }}
    elif method in ("session/new", "session/resume", "session/load"):
        assert (method == "session/new") == (turns == 0), method
        assert params.get("sessionId", NATIVE_ID) == NATIVE_ID
        assert params["systemPrompt"]["mode"] == "append"
        text = params["systemPrompt"]["content"][0]["text"]
        assert "SYSTEM HOOK MARKER" in text
        if turns < 2:
            label = ["first", "second"][turns]
            assert f"ACP instruction {label}" in text, text
        assert params["_meta"]["claudeCode"]["options"]["systemPrompt"] == {
            "type": "preset", "preset": "claude_code", "append": text, "snapshot": False,
        }
        if method == "session/new":
            result = {"sessionId": NATIVE_ID}
    elif method == "session/prompt":
        assert params["sessionId"] == NATIVE_ID
        blocks = params["prompt"]
        if turns < 2:
            label = ["first", "second"][turns]
            assert len(blocks) == 2 and blocks[0] == {
                "type": "text", "text": f"acp-hook-rewritten-{label}",
            }, blocks
            context = blocks[1]["text"]
            assert context.startswith("Agena input hook context\n\n"), context
            assert "SAVED_ACP_MEMORY_FIXTURE" in context and "MEMORY HOOK MARKER" in context, context
            answer = "ACP fixture accepted the input hooks"
        else:
            assert turns == 2 and blocks == [{"type": "text", "text": "/status"}], blocks
            answer = "ACP fixture kept the native command"
        turns += 1
        TURN_COUNT.write_text(str(turns))
        send(method="session/update", params={
            "sessionId": NATIVE_ID, "update": {"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": answer}},
        })
        result = {"stopReason": "end_turn"}
    elif method != "session/close":
        raise AssertionError(method)
    send(id=request["id"], result=result)
