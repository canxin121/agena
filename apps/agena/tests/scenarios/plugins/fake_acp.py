#!/usr/bin/env python3
"""ACP peer and Streamable HTTP MCP client for the external-agent system test."""

import base64
import hashlib
import json
import os
from pathlib import Path
import sys
import time
import traceback
import urllib.error
import urllib.request
import uuid

PROGRAM = Path(sys.argv[0]).name
assert PROGRAM in ("agy", "claude-agent-acp")
assert "AGENA_TEST_NEVER_FORWARD" not in os.environ
assert "CLAUDE_CODE_EXECUTABLE" not in os.environ

def record_exception(*error):
    Path(".acp-fixture-error").write_text("".join(traceback.format_exception(*error)))
    sys.__excepthook__(*error)


sys.excepthook = record_exception

if PROGRAM == "agy" and sys.argv[1:2] == ["--resume"]:
    _, resume, session, print_flag, prompt, output, output_format = sys.argv
    assert (resume, print_flag, prompt, output, output_format) == (
        "--resume", "--print", "/compact", "--output-format", "stream-json",
    )
    assert Path(f".acp-closed-{session}").is_file()
    Path(f".acp-compacted-{session}").touch()
    print(json.dumps({
        "event": "step_update", "step_update": {
            "conversation_id": session, "step_type": "checkpoint", "step_index": 4,
            "state": "DONE",
        },
    }))
    print(json.dumps({
        "event": "result", "result": {"conversation_id": session, "status": "SUCCESS"},
    }))
    sys.exit(0)
if PROGRAM == "agy":
    assert sys.argv[1:3] == ["acp", "--process-per-session"]
    assert "--gemini-dir" in sys.argv and "--agent" not in sys.argv
    assert "--include-default-customizations=false" not in sys.argv
else:
    assert sys.argv[1:] == []

client = urllib.request.build_opener(urllib.request.ProxyHandler({}))


def send(**fields):
    print(json.dumps({"jsonrpc": "2.0", **fields}), flush=True)


def session_update(**update):
    send(method="session/update", params={
        "sessionId": session,
        "update": update,
    })


def update(text):
    session_update(sessionUpdate="agent_message_chunk", content={"type": "text", "text": text})


def available_commands(*commands):
    session_update(sessionUpdate="available_commands_update", availableCommands=list(commands))


def form(identifier, message, properties, *, required=(), **scope):
    send(id=identifier, method="elicitation/create", params={
        **(scope or {"sessionId": session}),
        "mode": "form", "message": message,
        "requestedSchema": {"type": "object", "properties": properties, "required": list(required)},
    })


def receive():
    message = json.loads(sys.stdin.readline())
    assert message["jsonrpc"] == "2.0", message
    return message


def wait_fixture_file(name):
    deadline = time.monotonic() + 20
    while not Path(name).exists():
        assert time.monotonic() < deadline, f"timed out waiting for {name}"
        time.sleep(0.02)


def request_permission(identifier):
    send(id=identifier, method="session/request_permission", params={
        "sessionId": session,
        "toolCall": {"toolCallId": "native", "title": "Read fixture", "rawInput": {"path": "acp-input.txt"}},
        "options": [
            {"optionId": "approve", "kind": "allow_once", "name": "Allow"},
            {"optionId": "persistent", "kind": "allow_always", "name": "Always allow"},
            {"optionId": "reject", "kind": "reject_once", "name": "Deny"},
        ],
    })


mcp_session = None


def mcp(method, params, *, authorized=True, request_id=1):
    global mcp_session
    body = {"jsonrpc": "2.0", "method": method, "params": params}
    if request_id is not None:
        body["id"] = request_id
    headers = {
        "Accept": "application/json, text/event-stream",
        "Content-Type": "application/json",
    }
    if method != "initialize":
        headers["MCP-Protocol-Version"] = "2025-11-25"
        if mcp_session:
            headers["Mcp-Session-Id"] = mcp_session
    if authorized:
        headers.update(bridge_headers)
    request = urllib.request.Request(url, data=json.dumps(body).encode(), headers=headers)
    with client.open(request, timeout=30) as response:
        if method == "initialize":
            mcp_session = response.headers["Mcp-Session-Id"]
        if request_id is None:
            assert response.status == 202, response.status
            return None
        payload = response.read().decode()
        if response.headers.get_content_type() == "text/event-stream":
            payload = next(line[5:].strip() for line in payload.splitlines()
                           if line.startswith("data:") and line[5:].strip())
        return json.loads(payload)


def call_fixture(name, arguments):
    try:
        mcp("tools/list", {}, authorized=False)
        raise AssertionError("MCP accepted an unauthenticated connection")
    except urllib.error.HTTPError as error:
        assert error.code == 401, error.code
    mcp("initialize", {
        "protocolVersion": "2025-11-25", "capabilities": {},
        "clientInfo": {"name": PROGRAM, "version": "fixture"},
    })
    mcp("notifications/initialized", {}, request_id=None)
    listing = mcp("tools/list", {})["result"]["tools"]
    assert {tool["name"] for tool in listing} == {"plugins_list", "tools_list", "tools_help", "tools_call"}
    help = mcp("tools/call", {"name": "tools_help", "arguments": {"tool": name}})["result"]
    assert "Input schema:" in help["content"][0]["text"], help
    return mcp("tools/call", {
        "name": "tools_call", "arguments": {"tool": name, "input": arguments},
    })["result"]


def config_options():
    return [
        {"id": "native-model", "type": "select", "category": "model", "name": "Model",
         "currentValue": model, "options": [
             {"group": "native", "name": "Native", "options": [
                 {"value": "fast", "name": "Native Fast"}, {"value": "deep", "name": "Native Deep"},
             ]}
         ]},
        {"id": "native-thinking", "type": "select", "category": "thought_level", "name": "Thinking",
         "currentValue": thinking, "options": [
             {"value": value, "name": value.title()} for value in levels(model)
         ]},
        {"id": "mode", "type": "select", "category": "mode", "name": "Mode",
         "currentValue": "default", "options": [{"value": "default", "name": "Default"}]},
    ]


def levels(current_model):
    return ["low", "high"] if current_model == "deep" else ["low"]


session = None
model, thinking = "fast", "low"
probe = False
resumed = False
loaded = False
cancel_request = None
closed_for_compaction = None
for line in sys.stdin:
    request = json.loads(line)
    identifier = request.get("id")
    method = request["method"]
    params = request["params"]
    if method == "initialize":
        if params["protocolVersion"] == 2:
            send(id=identifier, result={"protocolVersion": 1})
            break
        assert params["protocolVersion"] == 1
        assert params["clientCapabilities"]["elicitation"]["form"] == {}
        assert params["clientCapabilities"]["plan"] == {}
        session_capabilities = {"resume": {}, "close": {}, "delete": {}}
        if PROGRAM == "claude-agent-acp":
            session_capabilities["options"] = {}
        result = {
            "protocolVersion": 1,
            "agentCapabilities": {
                "mcpCapabilities": {"http": True},
                "sessionCapabilities": session_capabilities,
                "loadSession": PROGRAM == "claude-agent-acp",
                "promptCapabilities": {
                    "image": not Path(".acp-no-image").exists(),
                    "audio": True,
                    "embeddedContext": True,
                },
            },
        }
    elif method == "session/options":
        assert PROGRAM == "claude-agent-acp" and session is None
        assert Path(params["cwd"]).resolve() == Path.cwd().resolve()
        assert "mcpServers" not in params
        Path(".acp-options-inspected").touch()
        result = {"configOptions": config_options()}
    elif method in ("session/new", "session/resume", "session/load"):
        assert Path(params["cwd"]).resolve() == Path.cwd().resolve()
        if params["mcpServers"] == []:
            assert method == "session/new"
            probe = True
            session = uuid.uuid4().hex
            Path(".acp-options-opened").touch()
            Path(f".acp-probe-{session}").touch()
            send(id=identifier, result={"sessionId": session, "configOptions": config_options()})
            continue
        assert len(params["mcpServers"]) == 1
        bridge = params["mcpServers"][0]
        assert (bridge["type"], bridge["name"]) == ("http", "agena")
        url = bridge["url"]
        bridge_headers = {header["name"]: header["value"] for header in bridge["headers"]}
        assert url.startswith("http://127.0.0.1:")
        assert bridge_headers["Authorization"].startswith("Bearer ")
        authorization_digest = hashlib.sha256(bridge_headers["Authorization"].encode()).hexdigest()
        if method == "session/resume" and PROGRAM == "claude-agent-acp":
            send(id=identifier, error={"code": -32601, "message": "Use session/load"})
            continue
        resumed = method != "session/new"
        loaded = method == "session/load"
        if resumed:
            session = params["sessionId"]
            assert Path(f".acp-{session}").is_file()
            if authorization_digest == Path(f".acp-{session}").read_text():
                assert PROGRAM == "agy" and closed_for_compaction == session
                assert Path(f".acp-compacted-{session}").is_file()
            closed_for_compaction = None
        else:
            session = uuid.uuid4().hex
        Path(f".acp-{session}").write_text(authorization_digest)
        if loaded:
            update("replayed reply must not enter the Agena transcript")
            if Path(".acp-plan-advance").exists():
                session_update(sessionUpdate="plan_update", plan={
                    "type": "markdown", "planId": "work", "content": "The plan restored by Claude.",
                })
                session_update(sessionUpdate="usage_update", used=550, size=2400)
        result = {"configOptions": config_options()}
        if not resumed:
            result["sessionId"] = session
        if not resumed and Path(".acp-catalog").is_file():
            # Claude publishes the catalog after answering session/new.
            send(id=identifier, result=result)
            if PROGRAM == "claude-agent-acp":
                available_commands(
                    {"name": "compact", "description": "Compact the native context"},
                    {"name": "mcp:repo:review", "description": "Review repository changes",
                     "input": {"hint": "<scope>"}},
                )
            else:
                available_commands({"name": "status", "description": "Antigravity status"})
            continue
    elif method == "session/set_config_option":
        assert params["sessionId"] == session
        value = params["value"]
        if params["configId"] == "native-model":
            assert value in ("fast", "deep"), value
            model = value
            if thinking not in levels(model):
                thinking = "low"
        elif params["configId"] == "native-thinking":
            assert value in levels(model), value
            thinking = value
        else:
            raise AssertionError("unrelated native configuration was sent")
        result = {"configOptions": config_options()}
    elif method in ("session/close", "session/delete"):
        assert params["sessionId"] == session
        if method == "session/delete":
            assert probe, "deleted an actual conversation"
            Path(f".acp-probe-{session}").unlink()
        elif not probe:
            assert PROGRAM == "agy" and Path(".acp-catalog").is_file()
            closed_for_compaction = session
            Path(f".acp-closed-{session}").touch()
        result = {}
    elif method == "session/prompt":
        assert not probe, "a configuration probe sent a prompt"
        assert params["sessionId"] == session
        assert not Path(".acp-no-image").exists(), "unsupported image reached the prompt"
        assert params["prompt"][0]["type"] == "text"
        prompt = params["prompt"][0]["text"]
        if prompt not in ("check composer", "check rich projection"):
            assert len(params["prompt"]) == 1, "Agena injected context into the prompt"
        if prompt == "reach native request limit":
            update("Partial native answer")
            send(id=identifier, result={"stopReason": "max_turn_requests"})
            continue
        elif prompt == "continue after native request limit":
            assert resumed
            update("Continued native answer")
        elif prompt == "check permission":
            update(f"{PROGRAM} started")
            session_update(sessionUpdate="tool_call", toolCallId="native",
                           title="Read fixture natively", name="Read", status="pending")
            request_permission("native-permission")
            decision = receive()
            assert decision["id"] == "native-permission"
            assert decision["result"]["outcome"] == {"outcome": "selected", "optionId": "approve"}
            content = [{"type": "content", "content": {"type": "text", "text": "native stdout"}}]
            session_update(sessionUpdate="tool_call_update", toolCallId="native",
                           content=content, status="in_progress")
            for _ in range(2):
                session_update(sessionUpdate="tool_call_update", toolCallId="native", status="completed")
            update("native permission accepted")
        elif prompt == "check elicitation":
            form("bad-schema", "Malformed form", {"obj": {"type": "object"}})
            response = receive()
            assert response["id"] == "bad-schema" and response["error"]["code"] == -32602, response
            form("question", "Choose execution settings.", {
                "region": {"type": "string", "oneOf": [
                    {"const": "eu-west", "title": "Europe", "description": "European endpoint"},
                    {"const": "us-east", "title": "US East"},
                ]},
                "tags": {"type": "array", "minItems": 2, "items": {"anyOf": [
                    {"const": "logs", "title": "Logs"},
                    {"const": "stats", "title": "Statistics"},
                ]}},
                "workers": {"type": "integer", "minimum": 1, "maximum": 8},
                "ratio": {"type": "number"},
                "dry_run": {"type": "boolean"},
                "environment": {"type": "string", "pattern": "^[a-z]+$"},
                "note": {"type": "string", "title": "Other"},
            }, required=["region", "tags", "workers", "ratio", "dry_run", "environment"])
            update("streamed while the form was open")
            response = receive()
            assert response["id"] == "question"
            assert response["result"] == {"action": "accept", "content": {
                "region": "eu-west", "tags": ["logs", "stats"], "workers": 4,
                "ratio": 0.25, "dry_run": False, "environment": "staging",
            }}, response
            update("received typed answers")
        elif prompt == "dismiss elicitation":
            form("dismiss", "Optional comment", {"note": {"type": "string"}}, requestId=identifier)
            response = receive()
            assert response["id"] == "dismiss" and response["result"] == {"action": "decline"}, response
            update("form declined")
        elif prompt == "withdraw elicitation":
            form(7, "A form the agent will withdraw", {"note": {"type": "string"}})
            wait_fixture_file(".acp-withdraw-form")
            request_permission("7")
            send(method="$/cancel_request", params={"requestId": 7})
            response = receive()
            assert response["id"] == 7 and response["result"] == {"action": "cancel"}, response
            update("form withdrawn and permission still waiting")
            response = receive()
            assert response["id"] == "7", response
            assert response["result"]["outcome"] == {"outcome": "selected", "optionId": "reject"}, response
            update("independent permission resolved")
        elif prompt == "withdraw permission":
            request_permission("permission-to-withdraw")
            wait_fixture_file(".acp-withdraw-permission")
            send(method="$/cancel_request", params={"requestId": "permission-to-withdraw"})
            response = receive()
            assert response["id"] == "permission-to-withdraw"
            assert response["result"]["outcome"] == {"outcome": "cancelled"}, response
            update("permission withdrawn")
        elif prompt == "cancel form turn":
            form("pending-at-cancel", "Cancel this turn", {"note": {"type": "string"}})
            request_permission("pending-permission-at-cancel")
            cancellation = receive()
            assert cancellation["method"] == "session/cancel" and cancellation["params"]["sessionId"] == session
            replies = {reply["id"]: reply["result"] for reply in (receive(), receive())}
            assert replies == {
                "pending-at-cancel": {"action": "cancel"},
                "pending-permission-at-cancel": {"outcome": {"outcome": "cancelled"}},
            }, replies
            Path(".acp-cancelled-pending").touch()
            result = {"stopReason": "cancelled"}
            send(id=identifier, result=result)
            continue
        elif prompt in ("read fixture", "deny fixture"):
            if PROGRAM == "claude-agent-acp":
                session_update(sessionUpdate="tool_call", toolCallId="bridge-mirror",
                               title="agena: tools_call", name="mcp__agena__tools_call", status="pending")
            output = call_fixture("fs.read", {"file_path": "acp-input.txt"})
            if PROGRAM == "claude-agent-acp":
                session_update(sessionUpdate="tool_call_update", toolCallId="bridge-mirror", status="completed")
            if prompt == "read fixture":
                assert not output.get("isError", False), output
                assert "acp callback content" in json.dumps(output), output
                update("MCP returned the file contents")
            else:
                assert output["isError"], output
                assert "acp callback content" not in json.dumps(output)
                update("MCP blocked by Agena policy")
        elif prompt == "check resume":
            assert resumed, "expected the native conversation to be resumed"
            assert len(params["prompt"]) == 1, "resume replayed prior messages"
            update("resumed")
        elif prompt == "check load":
            assert loaded, "expected the native conversation to be loaded"
            update("loaded")
        elif prompt == "check options":
            update(f"model {model}, thinking {thinking}")
        elif prompt == "/compact":
            assert PROGRAM == "claude-agent-acp" and not resumed
            send(method="session/update", params={
                "sessionId": "another-session", "update": {
                    "sessionUpdate": "available_commands_update",
                    "availableCommands": [{"name": "foreign", "description": "Foreign catalog"}],
                },
            })
            update("native compact received")
            wait_fixture_file(".acp-catalog-visible")
        elif prompt == '/mcp:repo:review  "scope"  all':
            assert loaded, "the native conversation was not loaded"
            available_commands({"name": "review", "description": "Updated command"})
            update("native review received exact arguments")
        elif prompt == "//agent:compact":
            assert loaded
            available_commands()
            update("escaped command received as text")
        elif prompt == "/status":
            assert PROGRAM == "agy" and not resumed
            update("native status received")
        elif prompt == "inspect after compacting":
            assert resumed and Path(f".acp-compacted-{session}").is_file()
            output = call_fixture("fs.read", {"file_path": "acp-input.txt"})
            assert "acp callback content" in json.dumps(output), output
            update("read through Agena after checkpoint")
        elif prompt == "show project plan":
            session_update(sessionUpdate="plan", entries=[
                {"content": "Legacy step", "priority": "low", "status": "pending"},
            ])
            session_update(sessionUpdate="plan_update", plan={
                "type": "items", "planId": "work", "entries": [
                    {"content": "Inspect workspace", "priority": "high", "status": "completed"},
                    {"content": "Refresh notes", "priority": "medium", "status": "in_progress"},
                ],
            })
            session_update(sessionUpdate="plan_update", plan={
                "type": "markdown", "planId": "obsolete", "content": "An older approach.",
            })
            session_update(sessionUpdate="usage_update", used=1200, size=4000,
                           cost={"amount": 12.5, "currency": "USD"})
            send(method="session/update", params={
                "sessionId": "another-session",
                "update": {"sessionUpdate": "usage_update", "used": 777, "size": 888},
            })
            update("plan visible before prompt completion")
            wait_fixture_file(".acp-plan-advance")
            session_update(sessionUpdate="plan", entries=[])
            session_update(sessionUpdate="plan_removed", planId="unknown")
            session_update(sessionUpdate="plan_removed", planId="obsolete")
            session_update(sessionUpdate="plan_update", plan={
                "type": "markdown", "planId": "work", "content": "A revised approach.",
            })
            session_update(sessionUpdate="plan_update", plan={
                "type": "items", "planId": "review", "entries": [
                    {"content": "Review result", "priority": "high", "status": "completed"},
                ],
            })
            session_update(sessionUpdate="usage_update", used=400, size=2000)
            session_update(sessionUpdate="usage_update", used=100, size=0)
            update("project plan revised")
            wait_fixture_file(".acp-plan-finish")
        elif prompt == "cancel plan progress":
            session_update(sessionUpdate="plan", entries=[
                {"content": "Stale if aborted", "priority": "high", "status": "in_progress"},
            ])
            session_update(sessionUpdate="usage_update", used=900, size=2000)
            update("waiting with a native plan")
            cancel_request = identifier
            continue
        elif prompt in ("check tool image", "deny tool image"):
            output = call_fixture("fs.read_media", {"path": "acp-image.png"})
            images = [block for block in output["content"] if block["type"] == "image"]
            if prompt == "check tool image":
                assert not output.get("isError", False), output
                assert len(images) == 1 and images[0]["mimeType"] == "image/png", output
                assert base64.b64decode(images[0]["data"]) == Path("acp-image.png").read_bytes()
                update("MCP returned the image")
            else:
                assert output["isError"] and not images, output
                update("MCP blocked the image")
        elif prompt == "check composer":
            text, image, path, skill = params["prompt"]
            assert text["text"] == "check composer"
            assert image == {
                "type": "image",
                "mimeType": "image/png",
                "data": base64.b64encode(Path("acp-image.png").read_bytes()).decode(),
            }
            assert path["type"] == "resource_link" and path["uri"] == Path("acp-input.txt").resolve().as_uri()
            assert skill["type"] == "text" and 'example@1/inspect' in skill["text"]
            update("image and references received")
        elif prompt == "check rich projection":
            _, resource, audio = params["prompt"]
            assert resource["type"] == "resource"
            assert resource["resource"]["text"] == Path("acp-input.txt").read_text()
            assert audio["type"] == "audio" and audio["mimeType"] == "audio/wav"
            assert base64.b64decode(audio["data"]) == Path("acp-audio.wav").read_bytes()
            child = "native-child"
            session_update(sessionUpdate="subagent_update", sessionId=child,
                           title="Research subagent", state={"state": "running"})
            send(method="session/update", params={"sessionId": child, "update": {
                "sessionUpdate": "session_message", "messageId": "incoming", "senderSessionId": session,
                "content": [{"type": "text", "text": "Inspect the fixture"}],
            }})
            send(method="session/update", params={"sessionId": child, "update": {
                "sessionUpdate": "agent_message_chunk", "messageId": "answer",
                "content": {"type": "text", "text": "Subagent response"},
            }})
            session_update(sessionUpdate="subagent_update", sessionId=child,
                           state={"state": "idle", "stopReason": "end_turn"})
            session_update(sessionUpdate="usage_update", used=71, size=128,
                           cost={"amount": 1.25, "currency": "credits"})
            session_update(sessionUpdate="notice", severity="warning", title="Native notice",
                           description="This notice does not enter the prompt.")
            session_update(sessionUpdate="compaction_update", compactionId="compact",
                           status="in_progress")
            session_update(sessionUpdate="compaction_summary_chunk", compactionId="compact",
                           content={"type": "text", "text": "Captured "})
            session_update(sessionUpdate="compaction_summary_chunk", compactionId="compact",
                           content={"type": "text", "text": "summary."})
            session_update(sessionUpdate="compaction_update", compactionId="compact", status="completed")
            session_update(sessionUpdate="tool_call", toolCallId="edit", kind="edit", title="Native edit",
                           status="in_progress", rawInput={"scope": "test"},
                           locations=[{"path": str(Path("acp-input.txt").resolve()), "line": 7}])
            session_update(sessionUpdate="tool_call_update", toolCallId="edit", status="completed",
                           rawOutput={"modified": True}, content=[
                               {"type": "diff", "path": "acp-input.txt", "oldText": "before\n", "newText": "after\n"},
                               {"type": "content", "content": {"type": "text", "text": "File changed"}},
                               {"type": "content", "content": {"type": "image", "mimeType": "image/png",
                                   "data": base64.b64encode(Path("acp-image.png").read_bytes()).decode()}},
                           ])
            update("rich projection accepted")
        elif prompt == "continue after cancellation":
            assert resumed, "lost the native session after cancellation"
            assert len(params["prompt"]) == 1, "resent a cancelled prompt"
            update("continued after cancellation")
        elif prompt == "check clear":
            assert not resumed and len(params["prompt"]) == 1
            update("cleared")
        elif prompt == "cancel this":
            Path(".acp-cancel-url").write_text(url)
            session_update(sessionUpdate="tool_call", toolCallId="waiting", title="Wait natively", status="in_progress")
            update("waiting for cancellation")
            cancel_request = identifier
            continue
        else:
            raise AssertionError(f"unexpected prompt {prompt!r}")
        result = {"stopReason": "end_turn"}
    elif method == "session/cancel":
        assert params["sessionId"] == session
        Path(".acp-cancelled").write_text("cancelled")
        send(id=cancel_request, result={"stopReason": "cancelled"})
        continue
    else:
        raise AssertionError(f"unexpected method {method!r}")
    send(id=identifier, result=result)
