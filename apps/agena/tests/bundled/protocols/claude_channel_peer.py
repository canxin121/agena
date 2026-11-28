"""A local Claude channel fixture with a trigger file and a one-time verdict."""
import json
import os
from pathlib import Path
import sys
import threading
import time

trigger = Path(sys.argv[1])
restart = trigger.with_suffix(".restart")
message_file = trigger.with_suffix(".message")
is_reconnect = restart.is_file()
output_lock = threading.Lock()
published = threading.Event()


def write(message):
    with output_lock:
        print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


def publish_notice():
    while not trigger.is_file():
        time.sleep(0.05)
    message = "fixture channel after reconnect" if is_reconnect else "fixture channel alert"
    if message_file.is_file():
        message = message_file.read_text(encoding="utf-8")
    published.set()
    write({
        "method": "notifications/claude/channel",
        "params": {
            "content": message,
            "meta": {"chat_id": os.environ["FIXTURE_ROOM"]},
        },
    })
    if not is_reconnect:
        while not restart.is_file():
            time.sleep(0.05)
        os._exit(0)


for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "initialize":
        write({
            "id": request["id"],
            "result": {
                "protocolVersion": "2025-11-25",
                "serverInfo": {"name": "fixture-channel", "version": "1.0"},
                "capabilities": {"tools": {}, "experimental": {
                    "claude/channel": {}, "claude/channel/permission": {},
                }},
                "instructions": "Channel notifications include a chat_id attribute.",
            },
        })
    elif method == "notifications/initialized":
        if is_reconnect:
            publish_notice()
        else:
            notice = threading.Thread(target=publish_notice, daemon=True)
            notice.start()
    elif method == "tools/list":
        write({"id": request["id"], "result": {"tools": [{
            "name": "reply",
            "description": "Reply to the fixture channel.",
            "inputSchema": {"type": "object", "properties": {
                "chat_id": {"type": "string"}, "text": {"type": "string"},
            }, "required": ["chat_id", "text"]},
        }]}})
    elif method == "tools/call" and request["params"]["name"] == "reply":
        arguments = request["params"]["arguments"]
        ready = published.is_set() and arguments["chat_id"] == os.environ["FIXTURE_ROOM"]
        message = "fixture reply: " + arguments["text"] if ready else "wrong channel connection"
        write({"id": request["id"], "result": {
            "isError": not ready,
            "content": [{"type": "text", "text": message}],
        }})
    elif method == "notifications/claude/channel/permission_request":
        write({
            "method": "notifications/claude/channel/permission",
            "params": {"request_id": request["params"]["request_id"], "behavior": "allow"},
        })
    elif "id" in request:
        write({"id": request["id"], "error": {"code": -32601, "message": "Unsupported fixture method"}})
