#!/usr/bin/env python3
"""Claude v1 AIR session-index recorder, isolated in the test workspace."""
import json
import sys
from pathlib import Path

cwd = Path.cwd()
archive = cwd / ".archive-current"
deleted = cwd / ".delete-old"
wire = cwd / ".index-wire"
active_id = "active"
archived_id = "archived"
foreign_id = "foreign"
busy_id = "busy"
subscription_id = "native-watch-7"


def send(**message):
    print(json.dumps({"jsonrpc": "2.0", **message}), flush=True)


def changes(identifier=subscription_id, sessions=(), removed=()):
    send(method="_session/list/changes", params={
        "subscriptionId": identifier,
        "sessions": list(sessions),
        "removed": list(removed),
    })


def row(identifier, indexed):
    is_archived = identifier == archived_id or (identifier == active_id and archive.exists())
    result = {
        "sessionId": identifier,
        "title": identifier.title(),
        "cwd": str(cwd.parent if identifier == foreign_id else cwd),
        "updatedAt": "2026-11-25T00:00:00Z",
    }
    if indexed:
        result["_meta"] = {"jetbrains": {"air": {
            "version": 1,
            "archived": is_archived,
            "state": "running" if identifier == busy_id else "idle",
            "model": "fixture",
        }}}
    return result


indexed = False
current_watch = None
version = 1
for raw in sys.stdin:
    request = json.loads(raw)
    if "id" not in request:
        continue
    method, params = request["method"], request["params"]
    identifier = request["id"]
    with wire.open('a') as log:
        log.write(f"{'index' if indexed else 'ordinary'} {method}\n")
    if method == "initialize":
        air = params.get("clientCapabilities", {}).get("_meta", {}).get("jetbrains", {}).get("air")
        indexed = "sessionIndex" in (air or {}).get("capabilities", [])
        version = 1 if indexed else int((cwd / ".ordinary-acp-version").read_text())
        if indexed:
            assert air == {"version": 1, "capabilities": ["sessionIndex"]}
        elif version == 1 and params["protocolVersion"] == 2:
            send(id=identifier, result={"protocolVersion": 1})
            break
        assert params["protocolVersion"] == version
        if version == 1:
            result = {
                "protocolVersion": 1,
                "agentCapabilities": {
                    "loadSession": True,
                    "sessionCapabilities": {"list": {}, "close": {}, "delete": {}},
                },
            }
        else:
            result = {"protocolVersion": 2, "capabilities": {"session": {"delete": {}}}}
        if indexed and (cwd / ".index-supported").exists():
            result["_meta"] = {"jetbrains": {"air": {"version": 1, "capabilities": [
                "sessionIndex", "sessionBatchGet", "sessionArchive", "sessionListSubscribe",
            ]}}}
    elif method == "session/list":
        assert Path(params["cwd"]) == cwd
        if indexed:
            assert (cwd / ".index-supported").exists()
            options = params["_meta"]["jetbrains"]["air"]
            assert options["version"] == 1 and options["list"]["includeWorktrees"] is False
            filter_name = options["list"]["archived"]
            assert filter_name in {"all", "archived", "unarchived"}
            all_rows = [row(id, True) for id in (active_id, archived_id, foreign_id, busy_id)]
            all_rows = [session for session in all_rows if session["sessionId"] != archived_id or not deleted.exists()]
            selected = [
                item for item in all_rows
                if filter_name == "all" or item["_meta"]["jetbrains"]["air"]["archived"] == (filter_name == "archived")
            ]
            cursor = params.get("cursor")
            if filter_name == "all" and cursor is None:
                result = {"sessions": selected[:1], "nextCursor": "indexed-page-2"}
            else:
                assert not cursor or (filter_name == "all" and cursor == "indexed-page-2")
                result = {"sessions": selected[1:] if cursor else selected}
        else:
            assert "_meta" not in params
            result = {"sessions": [row(active_id, False), row(foreign_id, False)]}
    elif method == "_session/batch_get":
        assert indexed and set(params) == {"sessionIds"}
        ids = params["sessionIds"]
        assert len(ids) == len(set(ids))
        result = {"sessions": [
            row(id, True) for id in ids
            if id in {active_id, archived_id, foreign_id, busy_id} and (id != archived_id or not deleted.exists())
        ]}
    elif method in {"_session/archive", "_session/unarchive"}:
        assert indexed and params == {"sessionId": active_id}
        if method == "_session/archive":
            archive.touch()
        else:
            archive.unlink(missing_ok=True)
        result = {}
    elif method == "_session/list/subscribe":
        assert indexed and params == {"cwd": str(cwd)}
        assert current_watch is None
        current_watch = subscription_id
        mode = (cwd / ".watch-mode").read_text()
        changes(sessions=[row(foreign_id, True)])
        changes("incorrect-subscription", sessions=[row(active_id, True)])
        if mode == "early":
            changes(sessions=[row(active_id, True)])
        send(id=identifier, result={"subscriptionId": subscription_id})
        if mode == "remove":
            changes(removed=[active_id])
        continue
    elif method == "_session/list/unsubscribe":
        assert indexed and params == {"subscriptionId": current_watch}
        current_watch = None
        result = {}
    elif method in {"session/load", "session/resume"}:
        assert not indexed and params["sessionId"] == archived_id and Path(params["cwd"]) == cwd
        if version == 2:
            assert method == "session/resume" and params["replayFrom"] == {"type": "start"}
        send(method="session/update", params={"sessionId": archived_id, "update": {
            "sessionUpdate": "user_message_chunk",
            "content": {"type": "text", "text": "User turn before archive"},
        }})
        if version == 2:
            send(method="session/update", params={"sessionId": archived_id, "update": {
                "sessionUpdate": "replay_complete",
            }})
        result = {}
    elif method == "session/close":
        assert not indexed and params == {"sessionId": archived_id}
        result = {}
    elif method == "session/delete":
        assert not indexed and params == {"sessionId": archived_id}
        assert air is None
        deleted.touch()
        result = {}
    else:
        raise AssertionError(method)
    send(id=identifier, result=result)
