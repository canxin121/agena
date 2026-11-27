#!/usr/bin/env python3
"""Exercise ACP v2 foreground/background, in-band MCP, and v1 client callbacks."""

import base64
import json
from pathlib import Path
import sys
import time
import traceback


def failure(*args):
    Path(".acp-transport-error").write_text("".join(traceback.format_exception(*args)))
    sys.__excepthook__(*args)


sys.excepthook = failure
antigravity = Path(sys.argv[0]).name == "agy"
version = 1 if antigravity or Path(".acp-force-v1").is_file() else 2
session = "native-transport"
request_id = 100_000


def send(**message):
    print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


def receive():
    message = json.loads(sys.stdin.readline())
    assert message["jsonrpc"] == "2.0", message
    return message


def update(update_type, **fields):
    send(method="session/update", params={
        "sessionId": session,
        "update": {"sessionUpdate": update_type, **fields},
    })


def say(text, message_id):
    content = {"type": "text", "text": text}
    if version == 2:
        update("agent_message", messageId=message_id, content=[content])
    else:
        update("agent_message_chunk", content=content)


def terminal_data(content):
    return base64.b64encode(content).decode("ascii")


def agena_tool(tool, arguments):
    return {"name": "tools_call", "arguments": {"tool": tool, "input": arguments}}


def foreign_reply():
    send(method="session/update", params={
        "sessionId": "unrelated-session",
        "update": {"sessionUpdate": "agent_message", "messageId": "answer",
                   "content": [{"type": "text", "text": "Foreign replacement"}]},
    })


image = {"type": "image", "mimeType": "image/png",
         "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGNgYPgPAAEDAQAIicLsAAAAAElFTkSuQmCC"}
audio = {"type": "audio", "mimeType": "audio/wav",
         "data": terminal_data(bytes.fromhex(
             "524946462600000057415645666d74201000000001000100401f0000803e000002001000"
             "64617461020000000000"))}


def continuation():
    return [
        {"type": "text", "text": "resume preview"},
        {"type": "resource_link", "name": "Native fixture",
         "uri": Path("native.txt").resolve().as_uri(), "mimeType": "text/plain",
         "size": Path("native.txt").stat().st_size},
        {"type": "text", "text": " with the linked fixture"},
    ]


def start_request(method, params):
    global request_id
    request_id += 1
    send(id=request_id, method=method, params=params)
    return request_id


def ask(method, params, *, expect_error=False):
    identifier = start_request(method, params)
    response = receive()
    assert response["id"] == identifier, response
    if expect_error:
        assert "error" in response, response
        return response["error"]
    assert "error" not in response, response
    return response["result"]


revision = {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}


def mcp_params(descriptor, method, params, **scope):
    return {
        "serverId": descriptor["serverId"],
        "requestId": f"test:{request_id + 1}",
        "method": method,
        "params": {**(params or {}), "_meta": revision},
        **scope,
    }


def mcp(descriptor, method, params=None, **scope):
    return ask("mcp/message", mcp_params(descriptor, method, params, **scope))


def wait(path):
    deadline = time.monotonic() + 20
    while not Path(path).is_file():
        assert time.monotonic() < deadline
        time.sleep(0.01)


def withdraw(identifier):
    send(method="$/cancel_request", params={"requestId": identifier})
    response = receive()
    assert response["id"] == identifier and response["error"]["code"] == -32800, response


def withdraw_tools(descriptor, prompt_id):
    file = str(Path("native.txt").resolve())
    scope = {"sessionId": session}
    if version == 2:
        send(id=prompt_id, result={"messageId": "withdraw-tool-request"})
        update("state_update", state="running")
        write = start_request("mcp/message", mcp_params(
            descriptor, "tools/call", agena_tool("fs.write", {
                "path": file, "content": "Unexpected write",
            }), **scope))
    else:
        write = start_request("fs/write_text_file", {**scope, "path": file, "content": "Unexpected write"})
    wait(".acp-withdraw-permission")
    withdraw(write)
    assert Path(file).read_text().startswith("native fixture first line")
    if version == 2:
        shell = start_request("mcp/message", mcp_params(
            descriptor, "tools/call", agena_tool("shell.exec", {
                "command": "touch .acp-shell-started; while [ ! -e .acp-shell-release ]; do sleep .05; done; touch .acp-shell-effect",
                "reads": [".acp-shell-release"],
                "writes": [".acp-shell-started", ".acp-shell-effect"],
            }), **scope))
        wait(".acp-shell-started")
        withdraw(shell)
        Path(".acp-shell-release").touch()
        read = mcp(descriptor, "tools/call", agena_tool("fs.read", {"file_path": file}), **scope)["result"]
        assert "native fixture first line" in json.dumps(read), read
    else:
        read = ask("fs/read_text_file", {**scope, "path": file})
        assert read["content"].startswith("native fixture first line"), read
    say("Another tool completed after the withdrawals", "kept-working")
    if version == 2:
        update("state_update", state="idle", stopReason="end_turn")
    else:
        send(id=prompt_id, result={"stopReason": "end_turn"})


resumed = False
setup_authorization = Path(".acp-setup-authorization").is_file()
resume_callbacks = Path(".acp-resume-callbacks").is_file()
resume_permission = None
resume_answer = None
for line in sys.stdin:
    message = json.loads(line)
    if resume_permission is not None and message.get("id") == resume_permission and "method" not in message:
        resume_answer = message
        continue
    method = message["method"]
    params = message["params"]
    identifier = message.get("id")
    if method == "initialize":
        if version == 1 and params["protocolVersion"] == 2:
            Path(".acp-v2-init-closed").touch()
            break
        assert params["protocolVersion"] == version, message
        if version == 2:
            assert params["info"]["name"] == "agena"
            assert params["capabilities"]["session"]["promptSuggestions"] == {}
            result = {
                "protocolVersion": 2,
                "info": {"name": "fixture", "version": "1"},
                "capabilities": {"session": {"mcp": {"acp": {}}, "prompt": {"image": {}}}},
            }
        else:
            assert Path(".acp-v2-init-closed").is_file()
            caps = params["clientCapabilities"]
            assert caps["fs"] == {"readTextFile": not antigravity, "writeTextFile": not antigravity}
            assert caps["terminal"] is (not antigravity)
            result = {"protocolVersion": 1, "agentCapabilities": {"mcpCapabilities": {"http": True}}}
            if not antigravity:
                result["_meta"] = {"steering": {"supported": True}}
        send(id=identifier, result=result)
    elif method in ("session/new", "session/resume"):
        assert Path(params["cwd"]) == Path.cwd(), params
        [descriptor] = params["mcpServers"]
        if version == 2:
            assert descriptor["type"] == "acp" and descriptor["name"] == "agena", descriptor
            discovery = mcp(descriptor, "server/discover")["result"]
            assert discovery["supportedVersions"] == ["2026-07-28"], discovery
            tools = mcp(descriptor, "tools/list")["result"]["tools"]
            assert {tool["name"] for tool in tools} == {"plugins_list", "tools_list", "tools_help", "tools_call"}
            update("available_commands_update", availableCommands=[
                {"type": "command", "name": "superseded", "description": "Before the setup response"},
            ])
            update("config_option_update", configOptions=[
                {"configId": "superseded", "name": "Earlier setting", "type": "boolean", "currentValue": False},
            ])
        else:
            assert descriptor["type"] == "http"
        if method == "session/resume":
            assert version == 2 and params["sessionId"] == session
            assert "replayFrom" not in params
            assert Path(".acp-session-v2").is_file()
            resumed = True
            resuming_edit = Path(".acp-parked-edit-sent").is_file() and not Path(".acp-later-edit-sent").is_file()
            if resuming_edit:
                say("Unattached reply during reconnect", "resume-only")
                update("agent_message", messageId="answer", content=[
                    {"type": "text", "text": "Reply revised on reconnect"},
                    audio,
                ])
                update("agent_message_chunk", messageId="answer",
                       content={"type": "text", "text": "Appended during resume"})
                foreign_reply()
            if resume_callbacks:
                call = agena_tool("fs.read", {"file_path": "native.txt"})
                blocked = ask("mcp/message", {
                    "serverId": descriptor["serverId"], "requestId": "resume-foreign",
                    "method": "tools/call", "sessionId": "another-session",
                    "params": {**call, "_meta": revision},
                }, expect_error=True)
                assert blocked["code"] == -32602
                result = mcp(descriptor, "tools/call", call, sessionId=session)["result"]
                assert "native fixture first line" in json.dumps(result), result
                update("state_update", state="requires_action")
                resume_permission = "resume-permission"
                send(id=resume_permission, method="session/request_permission", params={
                    "sessionId": session,
                    "title": "Approve recovered native work?",
                    "subject": {"type": "tool_call", "toolCall": {"toolCallId": "recovering-tool"}},
                    "options": [{"kind": "allow_once", "name": "Allow", "optionId": "resume-allow"}],
                })
            else:
                update("state_update", state="running" if resuming_edit else "idle")
            send(id=identifier, result={})
        else:
            if version == 2:
                Path(".acp-session-v2").touch()
            if setup_authorization:
                assert version == 1
                url = {
                    "mode": "url", "url": "https://example.invalid/setup",
                    "message": "Authorize this integration", "elicitationId": "setup-url",
                }
                assert ask("elicitation/create", {**url, "requestId": identifier + 1}) == {"action": "cancel"}
                assert ask("fs/read_text_file", {
                    "sessionId": session, "path": str(Path.cwd() / "native.txt"),
                }, expect_error=True)["code"] == -32800
                assert ask("elicitation/create", {**url, "requestId": identifier}) == {"action": "accept"}
                assert ask("elicitation/create", {**url, "requestId": identifier},
                           expect_error=True)["code"] == -32602
            result = {"sessionId": session}
            if version == 2:
                result["availableCommands"] = [
                    {"type": "command", "name": "setup-command", "description": "From the setup response",
                     "input": {"type": "_future_input", "hint": "Unsupported input"}},
                    {"type": "_future_command", "name": "unknown-command", "description": "Unrecognized command"},
                ]
                result["configOptions"] = [
                    {"configId": "setup-setting", "name": "Current setting", "type": "boolean", "currentValue": True},
                ]
            send(id=identifier, result=result)
    elif method == "session/prompt":
        assert params["sessionId"] == session
        prompt = params["prompt"][0]["text"]
        if prompt == "withdraw tool requests":
            withdraw_tools(descriptor, identifier)
            continue
        if prompt == "disconnect during a client write":
            assert version == 1
            start_request("fs/write_text_file", {
                "sessionId": session, "path": str(Path("native.txt").resolve()),
                "content": "Unexpected write",
            })
            wait(".acp-disconnect")
            sys.exit(0)
        if version == 2:
            assert params["idempotencyKey"], params
            if resume_callbacks:
                if resumed:
                    assert prompt == "continue recovered work"
                    answer = resume_answer if resume_answer is not None else receive()
                    assert answer == {
                        "jsonrpc": "2.0", "id": resume_permission,
                        "result": {"outcome": {"outcome": "selected", "optionId": "resume-allow"}},
                    }, answer
                else:
                    assert prompt == "start work to recover"
                # A resumed workflow may emit its final foreground state before
                # the reply to the prompt it accepted during that work.
                message_id = "recovered-user" if resumed else "priming-user"
                update("state_update", state="running")
                update("user_message", messageId=message_id, content=params["prompt"])
                say("Returned from resumed callbacks" if resumed else "Ready to resume", f"reply:{message_id}")
                update("state_update", state="idle", stopReason="end_turn")
                send(id=identifier, result={"messageId": message_id})
                continue
            if prompt == "resume edited message":
                update("state_update", state="running")
                update("state_update", state="idle", stopReason="max_tokens")
            send(id=identifier, result={"messageId": f"user:{identifier}"})
            update("state_update", state="running")
            if prompt == "start preview":
                assert not resumed
                say("Preview is still running", "checking")
                steer = receive()
                assert steer["method"] == "session/prompt"
                assert steer["params"]["prompt"][0]["text"] == "Please inspect the fixture"
                assert steer["params"]["idempotencyKey"] != params["idempotencyKey"]
                send(id=steer["id"], result={"messageId": f"user:{steer['id']}"})
                blocked = ask("mcp/message", {
                    "serverId": descriptor["serverId"], "requestId": "foreign",
                    "method": "tools/call", "sessionId": "another-session",
                    "params": {**agena_tool("fs.read", {"file_path": "native.txt"}), "_meta": revision},
                }, expect_error=True)
                assert blocked["code"] == -32602
                result = mcp(
                    descriptor, "tools/call", agena_tool("fs.read", {"file_path": "native.txt"}),
                    sessionId=session,
                )["result"]
                assert "native fixture first line" in json.dumps(result), result
                update("tool_call_update", toolCallId="job", title="Background", status="in_progress",
                       content=[{"type": "content", "content": {"type": "text", "text": "Superseded"}}])
                update("tool_call_update", toolCallId="job", content=[])
                for text in ("Live ", "preview."):
                    update("tool_call_content_chunk", toolCallId="job",
                           content={"type": "content", "content": {"type": "text", "text": text}})

                update("tool_call_content_chunk", toolCallId="job",
                       content={"type": "terminal", "terminalId": "build"})
                path = str(Path.cwd() / "native.txt")
                binary = str(Path.cwd() / "asset.png")
                copied = str(Path.cwd() / "asset-copy.png")
                update("tool_call_content_chunk", toolCallId="job", content={
                    "type": "diff",
                    "changes": [
                        {"operation": "modify", "path": path, "fileType": "text"},
                        {"operation": "copy", "oldPath": binary, "path": copied, "fileType": "binary"},
                    ],
                    "patch": {"format": "git_patch", "text":
                              f"diff --git {path} {path}\n--- {path}\n+++ {path}\n@@ -1 +1 @@\n-old\n+new\n"},
                })
                update("terminal_update", terminalId="build", command="build fixture", cwd=str(Path.cwd()),
                       output={"data": terminal_data(b"superseded output")})
                update("terminal_update", terminalId="build", output={"data": terminal_data(b"Terminal ")})
                for data in (b"\xe2", b"\x98\x83"):
                    update("terminal_output_chunk", terminalId="build", data=terminal_data(data))
                update("terminal_update", terminalId="build", exitStatus={"exitCode": 0})
                update("subagent_update", sessionId="native-terminal-child", title="Worker",
                       state={"state": "idle"})
                send(method="session/update", params={
                    "sessionId": "native-terminal-child",
                    "update": {"sessionUpdate": "terminal_output_chunk", "terminalId": "build",
                               "data": terminal_data(b"Worker output")},
                })
                for index in range(12):
                    update("compaction_update", compactionId=f"recent-{index}", status="completed",
                           summary=[{"type": "text", "text": "summary " * 768}])
                update("prompt_suggestions_update", suggestions=[
                    {"prompt": continuation()},
                    {"prompt": [{"type": "text", "text": "Incomplete suggestion"},
                                {"type": "audio", "data": "YQ==", "mimeType": "audio/wav"}]},
                ])
                update("state_update", state="idle", stopReason="end_turn",
                       background=[{"type": "tool_call", "toolCallId": "job"}])
                say("Foreground ended", "foreground")
                update("state_update", state="idle", background=None)
                say("Background is still pending", "background-pending")
                wait(".acp-finish-job")
                update("tool_call_update", toolCallId="job", status="completed")
                say("Background finished", "background")
                update("state_update", state="idle", background=[])
                wait(".acp-after-settlement")
                expired = ask("mcp/message", {
                    "serverId": descriptor["serverId"], "requestId": "expired",
                    "sessionId": session, "method": "tools/call",
                    "params": agena_tool("fs.read", {"file_path": "native.txt"}),
                }, expect_error=True)
                assert expired["code"] == -32602 and "expired" in expired["message"], expired
                update("available_commands_update", availableCommands=[
                    {"type": "command", "name": "passive", "description": "Published while idle"},
                ])
                update("prompt_suggestions_update", suggestions=[])
            elif prompt == "authorize native edit":
                call = "permission-edit"
                update("tool_call_update", toolCallId=call, title="Pending edit", status="pending",
                       content=[{"type": "content", "content": {"type": "text", "text": "Earlier preview"}}])
                path = str(Path.cwd() / "native.txt")
                permission = {
                    "title": "Approve the edit?",
                    "options": [{"kind": "allow_once", "name": "Allow", "optionId": "fixture-allow"}],
                    "subject": {"type": "tool_call", "toolCall": {
                        "toolCallId": call,
                        "content": [{
                            "type": "diff", "changes": [{"operation": "modify", "path": path}],
                            "patch": {"format": "git_patch", "text":
                                      f"diff --git {path} {path}\n--- {path}\n+++ {path}\n@@ -1 +1 @@\n-earlier\n+approved\n"},
                        }],
                    }},
                }
                assert ask("session/request_permission", {**permission, "sessionId": "foreign"}) == {
                    "outcome": {"outcome": "cancelled"},
                }
                allowed = ask("session/request_permission", {**permission, "sessionId": session})
                assert allowed == {"outcome": {"outcome": "selected", "optionId": "fixture-allow"}}, allowed
                update("tool_call_update", toolCallId=call, status="completed")
                say("Approved the edit", "approval-answer")
                update("state_update", state="idle", stopReason="end_turn")
            elif prompt == "edit the same message":
                update("agent_thought_chunk", messageId="thought", content={"type": "text", "text": "Old thought"})
                for content in [
                    {"type": "text", "text": "First "},
                    {"type": "text", "text": "draft"},
                    image,
                    {"type": "resource_link", "name": "Old link", "uri": "https://example.invalid/old"},
                ]:
                    update("agent_message_chunk", messageId="answer", content=content)
                update("tool_call_update", toolCallId="interleave", title="Between messages", status="completed")
                say("Other message", "other")
                update("agent_message_chunk", messageId="answer", content={"type": "text", "text": "Draft trailer"})
                wait(".acp-replace-message")
                update("agent_message", messageId="answer", content=[
                    {"type": "text", "text": "Final opener"},
                    {"type": "resource_link", "name": "New link", "uri": "https://example.invalid/new"},
                    audio,
                    {"type": "text", "text": "Final trailer"},
                ])
                update("agent_thought", messageId="thought", content=None)
                update("agent_message", messageId="other", content=[])
                update("agent_message", messageId="answer")
                say("Replacement observed", "signal")
                wait(".acp-finish-edit")
                update("agent_thought_chunk", messageId="thought", content={"type": "text", "text": "Continued thought"})
                update("agent_message_chunk", messageId="other", content={"type": "text", "text": "After reset"})
                update("state_update", state="idle", stopReason="end_turn")
                wait(".acp-edit-while-parked")
                update("agent_message", messageId="answer", content=[
                    {"type": "text", "text": "Reply revised while parked"},
                    image,
                ])
                update("agent_thought", messageId="thought", content=[])
                say("Unattached background reply", "idle-only")
                foreign_reply()
                Path(".acp-parked-edit-sent").touch()
                update("available_commands_update", availableCommands=[
                    {"type": "command", "name": "parked-edit", "description": "Parked edit sent"},
                ])
            elif prompt == "resume edited message":
                assert resumed
                say("Resume reached a new prompt", "signal-resume")
                wait(".acp-finish-resume-edit")
                update("agent_message_chunk", messageId="answer",
                       content={"type": "text", "text": "Appended in the second turn"})
                update("agent_thought_chunk", messageId="thought",
                       content={"type": "text", "text": "Thought continued after reconnect"})
                foreign_reply()
                Path(".acp-later-edit-sent").touch()
                update("state_update", state="idle", stopReason="end_turn")
            elif prompt == "continue after editing old history":
                assert resumed
                say("Third turn resumed", "third-turn")
                update("state_update", state="idle", usage={
                    "inputTokens": 13, "outputTokens": 7, "cachedReadTokens": 5, "totalTokens": 25,
                })
            else:
                assert prompt == "resume preview" and resumed
                assert params["prompt"] == continuation(), params["prompt"]
                say("Preview resumed", "resumed")
                update("state_update", state="idle", stopReason="end_turn")
        elif setup_authorization:
            assert prompt == "check setup authorization"
            say("Setup authorization accepted", "setup-complete")
            send(id=identifier, result={"stopReason": "end_turn"})
            wait(".acp-after-settlement")
            send(method="elicitation/complete", params={"elicitationId": "unknown"})
            send(method="elicitation/complete", params={"elicitationId": "setup-url"})
            # A repeated completion cannot create another notice, even after a different update.
            update("notice", severity="info", title="Idle fixture marker")
            send(method="elicitation/complete", params={"elicitationId": "setup-url"})
            update("available_commands_update", availableCommands=[
                {"name": "passive", "description": "Published while idle"},
            ])
        else:
            assert prompt == "client callbacks"
            say("Checking local callbacks", "callbacks-running")
            steer = receive()
            assert steer["method"] == "_session/steering" and steer["id"] != identifier, steer
            assert steer["params"] == {
                "sessionId": session,
                "prompt": [{"type": "text", "text": "Check the allowed file and terminal"}],
                "_meta": {"steering": {"idleBehavior": "promptRequired"}},
            }, steer
            send(id=steer["id"], result={"outcome": "accepted"})
            scoped = {"sessionId": session}
            path = str(Path.cwd() / "native.txt")
            denied = str(Path.cwd() / "private.txt")
            ask("fs/read_text_file", {**scoped, "path": denied}, expect_error=True)
            content = ask("fs/read_text_file", {**scoped, "path": path, "line": 2, "limit": 1})
            assert content == {"content": "native fixture second line\n"}, content
            ask("fs/write_text_file", {**scoped, "path": path, "content": "client rewrite\n"})
            assert Path(path).read_text() == "client rewrite\n"
            terminal = ask("terminal/create", {
                **scoped, "command": "printf", "args": ["%s", "output from ACP terminal"],
                "outputByteLimit": 1024,
            })
            terminal_scope = {**scoped, **terminal}
            assert ask("terminal/wait_for_exit", terminal_scope)["exitCode"] == 0
            output = ask("terminal/output", terminal_scope)
            assert "output from ACP terminal" in output["output"], output
            ask("terminal/release", terminal_scope)
            ask("terminal/output", terminal_scope, expect_error=True)
            say("File and terminal callbacks finished", "callbacks")
            send(id=identifier, result={"stopReason": "end_turn"})
    else:
        raise AssertionError(message)
