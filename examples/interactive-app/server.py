#!/usr/bin/env python3
"""Small real backend for the reply-embedded Flowboard example (stdlib only)."""
import argparse
import json
import sqlite3
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import urlsplit


def make_handler(database):
    def connect():
        connection = sqlite3.connect(database, timeout=5)
        connection.row_factory = sqlite3.Row
        return connection

    with connect() as db:
        db.execute("CREATE TABLE IF NOT EXISTS tasks (id INTEGER PRIMARY KEY, title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0)")
        db.execute("CREATE TABLE IF NOT EXISTS metadata (key TEXT PRIMARY KEY)")
        if not db.execute("SELECT 1 FROM metadata WHERE key = 'seeded'").fetchone():
            db.executemany("INSERT INTO tasks (title, done) VALUES (?, ?)", [
                ("Design the next chapter", 1), ("Connect the real backend", 1),
                ("Make something worth sharing", 0), ("Leave room for a little magic", 0),
            ])
            db.execute("INSERT INTO metadata VALUES ('seeded')")

    class Handler(BaseHTTPRequestHandler):
        def send_json(self, status, data):
            body = json.dumps(data, ensure_ascii=False).encode()
            self.send_response(status)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            path = urlsplit(self.path).path
            if path == "/health":
                return self.send_json(200, {"ok": True, "storage": "sqlite"})
            if path == "/tasks":
                with connect() as db:
                    tasks = [dict(row) for row in db.execute("SELECT id, title, done FROM tasks ORDER BY id DESC")]
                for task in tasks:
                    task["done"] = bool(task["done"])
                return self.send_json(200, {"tasks": tasks})
            self.send_json(404, {"error": "Not found"})

        def mutate(self, operation):
            path = urlsplit(self.path).path
            try:
                length = int(self.headers.get("Content-Length", "0"))
                if length < 0 or length > 8192:
                    return self.send_json(413, {"error": "Request too large"})
                body = json.loads(self.rfile.read(length)) if length else {}
                if not isinstance(body, dict):
                    raise ValueError("Expected an object")
                with connect() as db:
                    if operation == "create" and path == "/tasks":
                        title = body.get("title", "")
                        if not isinstance(title, str) or not title.strip() or len(title) > 200:
                            raise ValueError("Enter a task of 1–200 characters")
                        cursor = db.execute("INSERT INTO tasks (title) VALUES (?)", (title.strip(),))
                        result = {"id": cursor.lastrowid, "title": title.strip(), "done": False}
                    elif path.startswith("/tasks/") and path[7:].isdigit():
                        task_id = int(path[7:])
                        if operation == "update":
                            if not isinstance(body.get("done"), bool):
                                raise ValueError("done must be boolean")
                            cursor = db.execute("UPDATE tasks SET done = ? WHERE id = ?", (int(body["done"]), task_id))
                        else:
                            cursor = db.execute("DELETE FROM tasks WHERE id = ?", (task_id,))
                        if not cursor.rowcount:
                            return self.send_json(404, {"error": "Task not found"})
                        result = {"ok": True}
                    else:
                        return self.send_json(404, {"error": "Not found"})
                self.send_json(201 if operation == "create" else 200, result)
            except (ValueError, json.JSONDecodeError) as error:
                self.send_json(400, {"error": str(error)})

        def do_POST(self):
            self.mutate("create")

        def do_PATCH(self):
            self.mutate("update")

        def do_DELETE(self):
            self.mutate("delete")

    return Handler


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--database", default="flowboard.sqlite")
    args = parser.parse_args()
    database = Path(args.database).resolve()
    server = ThreadingHTTPServer(("127.0.0.1", args.port), make_handler(database))
    print(f"Flowboard ready on http://127.0.0.1:{args.port} (database: {database})", flush=True)
    server.serve_forever()
