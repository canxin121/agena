#!/usr/bin/env python3
"""Native lifecycle exercised through Agena's real ACP stdin/stdout connection."""
import base64
import fcntl
import json
from pathlib import Path
import sys
from time import monotonic, sleep
import uuid
import traceback


def send(**message):
    print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


def update(session, **value):
    send(method="session/update", params={"sessionId": session, "update": value})


def post_text(session, role, text, message_id):
    post_block(session, role, {"type": "text", "text": text}, message_id)


def post_block(session, role, content, message_id):
    update(
        session,
        sessionUpdate=f"{role}_message_chunk",
        content=content,
        **({"messageId": message_id} if message_id is not None else {}),
    )


def fork_cursor(session, message_id, cursor_id):
    update(
        session, sessionUpdate="cursor_update", cursorId=cursor_id,
        capabilities={"fork": {}},
        anchor={"type": "user_message", "messageId": message_id, "position": "before"},
    )


def log(method, session):
    with Path(".native-protocol.log").open("a") as output:
        output.write(f"{method} {session}\n")


def report_failure(kind, error, trace):
    frame = traceback.extract_tb(trace)[-1]
    log("fixture-failure", f"{kind.__name__} at line {frame.lineno}: {frame.line}")
    sys.__excepthook__(kind, error, trace)


sys.excepthook = report_failure
image = {
    "type": "image",
    "mimeType": "image/png",
    "data": "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAIAAACQd1PeAAAADElEQVR4nGNgYPgPAAEDAQAIicLsAAAAAElFTkSuQmCC",
}
wav = b"RIFF\x26\x00\x00\x00WAVEfmt \x10\x00\x00\x00\x01\x00\x01\x00\x40\x1f\x00\x00\x80\x3e\x00\x00\x02\x00\x10\x00data\x02\x00\x00\x00\x00\x00"
user_resources = [
    {"type": "resource_link", "name": "Saved link", "uri": "https://example.com/acp-history"},
    {"type": "audio", "mimeType": "audio/wav", "data": base64.b64encode(wav).decode()},
    {"type": "resource", "resource": {
        "uri": "file:///native/notes.txt", "mimeType": "text/plain", "text": "Archived resource\n",
    }},
    {"type": "resource", "resource": {
        "uri": "mcp://fixture/opaque.bin", "mimeType": "application/x-fixture",
        "blob": base64.b64encode(b"\x00\xff\x01").decode(),
    }},
]
cwd = Path.cwd()
version = int((cwd / ".native-protocol-version").read_text())
historical_fork = (cwd / ".historical-fork-support").is_file()
native_skills = (cwd / ".native-skills").is_file()
view_settings = (cwd / ".native-view-settings").is_file()
resume_errors = (cwd / ".native-resume-errors").is_file()
window_path = cwd / ".native-history-window"
history_window = window_path.read_text().strip() if window_path.is_file() else None
skills_disabled = (cwd / ".native-disable-skills").is_file()
lease = (cwd / ".native-single-connection").open("a") if (cwd / ".native-single-connection").exists() else None
skill_name = "summarize-fixture"
skill_path = cwd / ".agents" / "skills" / skill_name / "SKILL.md"
skill = {"type": "skill", "name": skill_name, "description": "Summarize the fixture", "path": str(skill_path)}
commands = [
    skill,
    {"type": "skill", "name": "missing-description"},
    {"type": "skill", "name": "builtin-skill", "description": "No local file", "path": "relative"},
    {"type": "command", "name": "status", "description": "Report saved status"},
]


def model_options(value="default"):
    return {"configOptions": [{
        "id": "model", "type": "select", "category": "model", "name": "Model",
        "currentValue": value, "options": [
            {"value": "default", "name": "Default"}, {"value": "retained", "name": "Retained"},
        ],
    }]}


current = None
prompted = False
for raw in sys.stdin:
    req = json.loads(raw)
    method, params = req["method"], req.get("params", {})
    if "id" not in req:
        continue
    session = params.get("sessionId", "")
    log(method, session)
    if lease is not None and method in {"session/load", "session/resume"}:
        try:
            fcntl.flock(lease, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            send(id=req["id"], error={"code": -32603, "message": "Native session is active elsewhere"})
            continue
    if method in {"session/new", "session/load", "session/resume", "session/fork"}:
        options = params["_meta"]["claudeCode"]["options"]
        assert options["tools"] == [] and options["strictMcpConfig"] is True
        assert options["settingSources"] == []
        assert "restricted" in options["extraArgs"]
        assert "disable-slash-commands" not in options["extraArgs"]
    if method == "initialize":
        if version == 1 and params["protocolVersion"] == 2:
            send(id=req["id"], result={"protocolVersion": 1})
            break
        assert params["protocolVersion"] == version, params
        lifecycle = {
            key: {} for key in (
                "delete", "fork", "title", "status", "additionalDirectories", "systemPrompt",
            )
        }
        if version == 2:
            result = {
                "protocolVersion": 2,
                "info": {"name": "lifecycle-fixture"},
                "capabilities": {"session": {"mcp": {"http": {}}, "prompt": {"image": {}}, **lifecycle}},
            }
            if native_skills:
                assert params["capabilities"]["session"]["commands"]["skill"] == {}
                if not skills_disabled:
                    result["capabilities"]["session"]["prompt"]["skill"] = {}
        else:
            lifecycle.update(list={}, resume={}, close={})
            if resume_errors:
                del lifecycle["resume"]
            result = {
                "protocolVersion": 1,
                "agentCapabilities": {
                    "loadSession": True,
                    "mcpCapabilities": {"http": True},
                    "promptCapabilities": {"image": True},
                    "sessionCapabilities": lifecycle,
                },
            }
    elif method == "session/list":
        assert Path(params["cwd"]) == cwd
        next_cursor = "" if version == 2 else "page2"
        if params.get("cursor") == next_cursor:
            result = {
                "sessions": [{
                    "sessionId": "native-project",
                    "cwd": str(cwd),
                    "title": "Unaffiliated native conversation",
                    "updatedAt": "2026-11-25T00:00:00Z",
                }]
            }
        else:
            result = {
                "sessions": [{"sessionId": "foreign", "cwd": str(cwd.parent)}],
                "nextCursor": next_cursor,
            }
    elif (method == "session/load" and not params["mcpServers"]) or (
        method == "session/resume" and params.get("replayFrom") == {"type": "start"}
    ):
        assert method == ("session/load" if version == 1 else "session/resume"), params
        assert session == "native-project" and Path(params["cwd"]) == cwd
        assert params["mcpServers"] == []
        started = cwd / ".native-first-replay-started"
        if (cwd / ".native-hold-first-replay").is_file() and not started.is_file():
            started.touch()
            deadline = monotonic() + 20
            while not (cwd / ".native-release-replay").is_file():
                assert monotonic() < deadline, "timed out waiting to release native replay"
                sleep(0.02)
        replay_override = cwd / ".native-replay-override"
        if replay_override.is_file():
            assert version == 2
            mode = replay_override.read_text()
            post_text(session, "user", "Markerless native history", "markerless")
            if mode == "eof":
                break
            if mode == "error":
                send(id=req["id"], error={"code": -32603, "message": "Replay interrupted"})
            else:
                assert mode == "success"
                update(session, sessionUpdate="state_update", state="idle")
                send(id=req["id"], result={})
            continue
        if view_settings and (cwd / ".native-expect-reopen-settings").is_file():
            assert params["additionalDirectories"] == [str(cwd / "extra")]
            assert any("Retain my custom Agena prompt" in block.get("text", "")
                       for block in params["systemPrompt"]["content"])
            log("replayed-with-custom-context", session)
        current = session
        if version == 2:
            post_text(session, "user", "Obsolete user", "user-1")
            applied_skills = [{"type": "skill", "name": skill_name}] if native_skills else []
            update(
                session, sessionUpdate="user_message", messageId="user-1",
                content=[*applied_skills, {"type": "text", "text": "Imported user"}, *user_resources],
            )
        else:
            post_text(session, "user", "Imported user", "user-1")
            for block in user_resources:
                post_block(session, "user", block, "user-1")
        update(
            session, sessionUpdate="agent_thought_chunk", messageId="assistant-1",
            content={"type": "text", "text": "Imported reasoning"},
        )
        post_text(session, "agent", "Imported ", "assistant-1")
        update(
            session, sessionUpdate="tool_call", toolCallId="edit", title="Historical edit",
            kind="edit", status="in_progress", rawInput={"path": "notes.txt"},
            content=[{"type": "content", "content": {"type": "text", "text": "Obsolete result"}}],
        )
        if version == 2:
            post_text(session, "agent", "Obsolete second segment", "assistant-1")
            update(session, sessionUpdate="agent_message", messageId="assistant-1", content=[])
            post_text(session, "agent", "Obsolete after clear", "assistant-1")
            update(session, sessionUpdate="agent_message", messageId="assistant-1",
                   content=[{"type": "text", "text": "Imported assistant"}, image])
            update(session, sessionUpdate="agent_message", messageId="assistant-1",
                   _meta={"fixture": {"note": "Metadata-only update"}})
        else:
            post_text(session, "agent", "assistant", "assistant-1")
            post_block(session, "agent", image, "assistant-1")
        diff = {"type": "diff", "path": "notes.txt", "oldText": "before\n", "newText": "after\n"}
        if version == 2:
            path = str(cwd / "notes.txt")
            diff = {
                "type": "diff", "changes": [{"operation": "modify", "path": path, "fileType": "text"}],
                "patch": {"format": "git_patch", "text":
                          f"diff --git {path} {path}\n--- {path}\n+++ {path}\n@@ -1 +1 @@\n-before\n+after\n"},
            }
        update(
            session, sessionUpdate="tool_call_update", toolCallId="edit", status="completed",
            locations=[{"path": "notes.txt", "line": 1}], rawOutput={"saved": True},
            content=[
                {"type": "content", "content": {"type": "text", "text": "Saved edit"}},
                diff,
                {"type": "content", "content": image},
            ],
        )
        post_text("foreign", "agent", "Unrelated output", "ignored")
        if version == 2:
            child = "native-replay-child"
            nested = "native-replay-descendant"
            update(session, sessionUpdate="subagent_update", sessionId=child, title="Imported child")
            post_text(child, "agent", "Child history", "assistant-1")
            update(child, sessionUpdate="subagent_update", sessionId=nested, title="Imported descendant")
            update(child, sessionUpdate="replay_complete")
            update(child, sessionUpdate="state_update", state="idle", stopReason="end_turn")
            post_text(child, "agent", " and a live continuation", "assistant-1")
            update("foreign", sessionUpdate="subagent_update", sessionId="foreign-child", title="Foreign child")
            post_text("foreign-child", "agent", "Foreign child output", "assistant-1")
            if historical_fork:
                fork_cursor(session, "stored-user-2", "fixture/opaque:imported")
                update(session, sessionUpdate="user_message", messageId="stored-user-2",
                       content=[{"type": "text", "text": "Imported cursor target"}])
                post_text(session, "agent", "Reply to imported cursor target", "stored-agent-2")
            for index in range(12):
                update(session, sessionUpdate="compaction_update", compactionId=f"import-{index}",
                       status="completed", summary="R" * 6144)
        outside = cwd / ".native-outside-text"
        if outside.is_file():
            post_text(session, "user", "From the agent CLI", "outside-user")
            post_text(session, "agent", outside.read_text(), "outside-answer")
        if history_window in {"", "wire"}:
            count = 5 if history_window == "wire" else 3
            for number in range(1, count + 1):
                user_id = f"window-user-{number}"
                fork_cursor(session, user_id, f"fixture/window/{number}")
                post_text(session, "user", f"Window turn {number}", user_id)
                post_text(session, "agent", str(number) * (6 * 1024 * 1024), f"window-answer-{number}")
            if history_window == "wire":
                update(session, sessionUpdate="agent_message", messageId="window-answer-1",
                       content=[{"type": "text", "text": "Late evicted reply"}])
                update(session, sessionUpdate="tool_call_update", toolCallId="edit", title="Late evicted tool")
        elif history_window in {"records", "updates"}:
            user_id = "over-limit-user" if version == 2 else None
            post_text(session, "user", "Oversized native turn", user_id)
            if history_window == "records":
                for number in range(6100):
                    post_text(session, "agent", "x", f"over-limit-{number}")
            else:
                for _ in range(65800):
                    post_text(session, "agent", "x", None)
            user_id = "latest-user" if version == 2 else None
            answer_id = "latest-answer" if version == 2 else None
            post_text(session, "user", "Latest native turn", user_id)
            post_text(session, "agent", "Latest native reply", answer_id)
        elif history_window == "latest":
            post_text(session, "user", "Newest native question", "newest-user")
            post_text(session, "agent", "Old beginning\n" + '\t"你' * 2500000 + "\nNewest native answer", "newest-answer")
            update(session, sessionUpdate="tool_call", toolCallId="newest-tool", title="Newest native tool",
                   status="completed")
        elif history_window == "media":
            update(session, sessionUpdate="user_message", messageId="media-user", content=[
                {"type": "text", "text": "Attachment overflow"},
                {"type": "resource", "resource": {"uri": "mcp://fixture/large.txt", "text": "x" * (9 * 1024 * 1024)}},
                *[user_resources[-1]] * 17,
            ])
            post_text(session, "agent", "Reply survives attachment omission", "media-answer")
        elif history_window == "preview":
            for number in range(1, 13):
                user_id = f"preview-user-{number}" if version == 2 else None
                answer_id = f"preview-answer-{number}" if version == 2 else None
                post_text(session, "user", f"Preview turn {number}", user_id)
                post_text(session, "agent", f"Preview answer {number}", answer_id)
            if version == 2:
                update(session, sessionUpdate="agent_message", messageId="preview-answer-4",
                       content=[{"type": "text", "text": "Updated earlier answer"}])
        if version == 2:
            update(session, sessionUpdate="replay_complete")
            post_text(nested, "agent", "Descendant history", "assistant-1")
            update(nested, sessionUpdate="terminal_update", terminalId="child-terminal",
                   output={"data": base64.b64encode(b"Historical child terminal\n").decode()})
            update(nested, sessionUpdate="replay_complete")
            update(nested, sessionUpdate="state_update", state="idle", stopReason="end_turn")
            if historical_fork:
                fork_cursor(session, "user-1", "fixture/too-late")
            update(session, sessionUpdate="state_update", state="idle")
            post_text(session, "agent", "Live output after replay", "live")
            update(session, sessionUpdate="tool_call", toolCallId="live", title="Live tool", status="pending")
        update(
            session, sessionUpdate="session_info_update", title="Native history",
            updatedAt="2026-11-25T00:00:00Z",
        )
        result = {"availableCommands": commands} if native_skills else model_options() if view_settings else {}
    elif method in {"session/resume", "session/load"}:
        assert session in {"native-project", "native-fork", "native-fork-imported", "native-fork-live"}
        assert Path(params["cwd"]) == cwd
        assert len(params["mcpServers"]) == 1
        if resume_errors:
            error = (cwd / ".native-resume-errors").read_text().strip()
            if error:
                if error == "transient":
                    code, data = -32603, {}
                else:
                    code = -32002
                    data = {"sessionId": session if error == "missing" else "different-native-id"}
                send(id=req["id"], error={"code": code, "message": "Fixture refuses resume", "data": data})
                continue
        if session == "native-project" and not (historical_fork or native_skills or resume_errors or history_window is not None):
            assert params["additionalDirectories"] == [str(cwd / "extra")], params
            system_prompt = params["systemPrompt"]
            assert system_prompt["mode"] == "append"
            assert any(
                block["type"] == "text" and "Use the workspace lifecycle instructions." in block["text"]
                for block in system_prompt["content"]
            )
        current = session
        result = {"availableCommands": commands} if native_skills else model_options() if view_settings else {}
    elif method == "session/new":
        current = "native-new"
        result = {"sessionId": current, **(model_options() if view_settings else {})}
    elif method == "session/set_config_option" and view_settings:
        assert session == current and params["configId"] == "model", params
        result = model_options(params["value"])
    elif method == "session/prompt":
        assert session == current
        prompted = True
        if params["prompt"][0].get("text") == "check in-memory upload":
            assert params["prompt"][1] == image, params["prompt"]
        if version == 2:
            prompt_text = "".join(block.get("text", "") for block in params["prompt"])
            message_id = f"user:{req['id']}"
            if native_skills and not skills_disabled:
                assert params["prompt"] == [
                    {"type": "skill", "name": skill_name},
                    {"type": "text", "text": "Summarize these notes"},
                ], params
            elif native_skills:
                assert params["prompt"] == [
                    {"type": "text", "text": f"/{skill_name} legacy arguments"},
                ], params
            if historical_fork and prompt_text == "Live cursor target":
                message_id = "live-user-3"
                fork_cursor(session, message_id, "fixture/opaque:live")
            elif historical_fork and prompt_text == "Continue from imported cut":
                message_id = "forked-user-2"
                fork_cursor(session, message_id, "fixture/opaque:foreign")
            elif historical_fork and prompt_text == "Unmatched cursor":
                fork_cursor(session, "not-the-acknowledged-message", "fixture/wrong-message")
                fork_cursor("foreign", message_id, "fixture/wrong-session")
            send(id=req["id"], result={"messageId": message_id})
            update(session, sessionUpdate="state_update", state="running")
        update(session, sessionUpdate="session_info_update", title="Native generated title")
        if native_skills:
            if skills_disabled:
                update(session, sessionUpdate="available_commands_update", availableCommands=commands)
                post_text(session, "agent", "Text fallback accepted", "skill-fallback")
            else:
                update(session, sessionUpdate="available_commands_update", availableCommands=[
                    {"type": "skill", "name": "live-skill", "description": "Added during the turn"},
                ])
                post_text(session, "agent", "Structured skill accepted", "skill-invocation")
        else:
            post_text(session, "agent", f"Reply from {session}", str(uuid.uuid4()))
        if version == 2:
            update(session, sessionUpdate="state_update", state="idle", stopReason="end_turn")
            continue
        result = {"stopReason": "end_turn"}
    elif method == "session/fork":
        assert session == "native-project" and Path(params["cwd"]) == cwd
        if historical_fork:
            cursor = params["cursorId"]
            log("fork-cursor", cursor)
            result = {"sessionId": {
                "fixture/opaque:imported": "native-fork-imported",
                "fixture/opaque:live": "native-fork-live",
            }[cursor]}
        else:
            assert "cursorId" not in params
            assert params["additionalDirectories"] == [str(cwd / "extra")], params
            assert params["systemPrompt"]["mode"] == "append"
            result = {"sessionId": "native-fork"}
    elif method == "session/status":
        assert session in {"native-project", "native-fork", "native-new", "native-fork-imported", "native-fork-live"}
        result = {"status": "saved"}
    elif method == "session/set_title":
        assert session == "native-project"
        result = {"title": f"Remote {params['title']}"}
    elif method == "session/close":
        assert session in {"native-project", "native-fork", "native-new", "native-fork-imported", "native-fork-live"}
        if current == session:
            if prompted:
                log("close-prompted", session)
            current = None
            if lease is not None:
                fcntl.flock(lease, fcntl.LOCK_UN)
        result = {}
    elif method == "session/delete":
        assert session == "native-fork" or (view_settings and session == "native-new")
        result = {}
    else:
        raise AssertionError(f"Unexpected ACP method: {method}")
    send(id=req["id"], result=result)
