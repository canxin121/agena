#!/usr/bin/env python3
"""Print a complete assistant-reply fence for this example."""
import json
from pathlib import Path

manifest = {
    "id": "flowboard",
    "title": "Flowboard",
    "description": "A focused task board with a real Python + SQLite backend",
    "height": 640,
    "html": Path(__file__).with_name("index.html").read_text(),
    "services": [{"name": "api", "command": "python3", "args": ["server.py", "--port", "{port}"], "cwd": "examples/interactive-app", "port": 0, "healthPath": "/health"}],
}
if __name__ == "__main__":
    print("```agena-ui\n" + json.dumps(manifest, ensure_ascii=False) + "\n```")
