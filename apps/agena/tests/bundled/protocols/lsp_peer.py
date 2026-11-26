"""Local LSP peer used by Rust system tests through Agena's normal LSP launcher.

This is a protocol fixture, not a test runner. It has no network access,
third-party imports, user configuration, or command execution.
"""
import json
import os
import re
import sys
from pathlib import Path

log_path = Path(sys.argv[1])
documents = {}
request_count = 0


def record(value):
    with log_path.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(value, ensure_ascii=False) + "\n")


def send(value):
    body = json.dumps(value, ensure_ascii=False).encode("utf-8")
    header = ("Content-Length: %d\r\n\r\n" % len(body)).encode("ascii")
    sys.stdout.buffer.write(header + body)
    sys.stdout.buffer.flush()


def respond(request, value):
    send({"jsonrpc": "2.0", "id": request["id"], "result": value})


def diagnostics(uri):
    document = documents[uri]
    send({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "version": document["version"],
            "diagnostics": [{
                "range": {
                    "start": {"line": 0, "character": 0},
                    "end": {"line": 0, "character": 3},
                },
                "severity": 2,
                "source": "local-system-fixture",
                "message": "lsp-diagnostic-version-%d" % document["version"],
            }],
        },
    })


record({
    "event": "started",
    "pid": os.getpid(),
    "shared_env": os.environ.get("AGENA_LSP_SHARED"),
    "server_env": os.environ.get("AGENA_LSP_SERVER"),
})

while True:
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            record({"event": "stdin-eof"})
            sys.exit(0)
        if line in (b"\r\n", b"\n"):
            break
        key, value = line.decode("ascii").split(":", 1)
        headers[key.strip().lower()] = value.strip()
    length = int(headers["content-length"])
    if not 0 < length <= 1024 * 1024:
        raise ValueError("LSP fixture frame exceeded its bound")
    body = sys.stdin.buffer.read(length)
    if len(body) != length:
        raise EOFError("incomplete LSP frame")
    request = json.loads(body)
    request_count += 1
    if request_count > 128:
        raise RuntimeError("LSP fixture request budget exceeded")
    record(request)
    method = request["method"]
    params = request.get("params") or {}

    if method == "initialize":
        respond(request, {
            "capabilities": {
                "positionEncoding": "utf-16",
                "textDocumentSync": {"openClose": True, "change": 2, "save": {"includeText": True}},
                "hoverProvider": True,
                "definitionProvider": True,
                "referencesProvider": True,
                "implementationProvider": True,
                "documentSymbolProvider": True,
                "workspaceSymbolProvider": True,
            },
            "serverInfo": {"name": "local-system-lsp", "version": "1.0"},
        })
    elif method == "textDocument/didOpen":
        document = params["textDocument"]
        documents[document["uri"]] = {
            "text": document["text"],
            "version": document["version"],
        }
        diagnostics(document["uri"])
    elif method == "textDocument/didChange":
        document = params["textDocument"]
        changes = params["contentChanges"]
        previous = re.split(r"\r\n|\r|\n", documents[document["uri"]]["text"])
        expected_range = {
            "start": {"line": 0, "character": 0},
            "end": {"line": len(previous) - 1, "character": len(previous[-1].encode("utf-16-le")) // 2},
        }
        if len(changes) != 1 or changes[0].get("range") != expected_range:
            raise ValueError("the fixture negotiated incremental synchronization")
        documents[document["uri"]] = {
            "text": changes[0]["text"],
            "version": document["version"],
        }
        diagnostics(document["uri"])
    elif method == "textDocument/hover":
        document = documents[params["textDocument"]["uri"]]
        if "force_peer_exit" in document["text"]:
            record({"event": "forced-exit"})
            sys.exit(0)
        marker = "updated" if "updated_symbol" in document["text"] else "initial"
        respond(request, {
            "contents": {
                "kind": "markdown",
                "value": "lsp-hover-%s; version=%d" % (marker, document["version"]),
            }
        })
    elif method in ("textDocument/definition", "textDocument/references", "textDocument/implementation"):
        uri = params["textDocument"]["uri"]
        locations = [{
            "uri": uri,
            "range": {
                "start": {"line": 0, "character": 4},
                "end": {"line": 0, "character": 18},
            },
        }]
        if method == "textDocument/references":
            locations.append({
                "uri": uri,
                "range": {
                    "start": {"line": 1, "character": 0},
                    "end": {"line": 1, "character": 14},
                },
            })
        if method == "textDocument/definition":
            respond(request, [{
                "targetUri": uri,
                "targetRange": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 18}},
                "targetSelectionRange": locations[0]["range"],
            }])
        else:
            respond(request, locations)
    elif method == "textDocument/documentSymbol":
        respond(request, [{
            "name": "initial_symbol",
            "kind": 13,
            "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 18}},
            "selectionRange": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 18}},
        }])
    elif method == "workspace/symbol":
        respond(request, [{
            "name": "initial_symbol",
            "kind": 13,
            "location": {
                "uri": next(iter(documents)),
                "range": {"start": {"line": 0, "character": 4}, "end": {"line": 0, "character": 18}},
            },
        }])
    elif method == "textDocument/didClose":
        documents.pop(params["textDocument"]["uri"], None)
    elif method == "shutdown":
        respond(request, None)
    elif method == "exit":
        record({"event": "exit-notification"})
        sys.exit(0)
    elif "id" in request:
        send({
            "jsonrpc": "2.0", "id": request["id"],
            "error": {"code": -32601, "message": "unsupported local LSP fixture method"},
        })
