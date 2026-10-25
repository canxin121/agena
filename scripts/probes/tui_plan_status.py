#!/usr/bin/env python3
"""Verify session plan chips and live updates in an isolated real terminal."""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import socket
import subprocess
import time
import traceback
import unicodedata

import http_experience as fixture
import tui_experience as tui


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    trace = fixture.JsonLines(out / "http.jsonl")
    log = (out / "server.log").open("wb")
    result = {"status": "failed", "binary_sha256": fixture.file_digest(binary), "cases": []}
    process = client = None
    tui.fixture = fixture
    try:
        workspace = fixture.prepare_configuration(out, "http://127.0.0.1:1/v1")
        env = fixture.clean_environment(out)
        env["AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES"] = "1"
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        api = fixture.Api(f"http://127.0.0.1:{port}", 20, trace)

        def start_server():
            server = subprocess.Popen(
                [str(binary), "--data-dir", str(out / "data"), "server", "--host", "127.0.0.1",
                 "--port", str(port), "--workspace", str(workspace)],
                cwd=out / "process-cwd", env=env, stdout=log, stderr=subprocess.STDOUT,
            )
            record = out / "data/run/server.json"

            def published():
                fixture.require(server.poll() is None, "server exited before publishing its identity")
                try:
                    return json.loads(record.read_text()).get("pid") == server.pid
                except (OSError, ValueError):
                    return False

            try:
                fixture.poll(published, api.timeout, "server did not replace the old discovery record")
                fixture.wait_server(server, api, record)
            except BaseException:
                server.terminate()
                server.wait(timeout=10)
                raise
            return server

        def tool(session_id, name, input=None):
            response = api.call("POST", "/api/v1/plugins/tools/invoke", {
                "plugin_id": "agena.plan", "tool": name, "input": input or {},
                "session_id": session_id,
            })[0]
            fixture.require(response["status"] == "completed", str(response))
            return response

        def checkpoint(label):
            fixture.require(not client.failure, str(client.failure))
            (client.directory / f"{label}.txt").write_text(client.text())
            result["cases"].append(label)

        def border():
            return next((line for line in reversed(client.text().splitlines()) if "└" in line), "")

        def progress(label, checkpoint_label):
            fixture.poll(lambda: label in border(), 15,
                         f"plan chip did not converge to {label!r}; screen={client.text()}")
            line = border()
            fixture.require(line.index(label) - line.index("└") <= 3,
                            f"plan chip is not in the bottom-left corner: {line}")
            checkpoint(checkpoint_label)

        def switch_to(title):
            client.send_prompt("/hub")
            client.wait_text("Ctrl+N new session", f"hub-for-{title}")
            client.send(b"/" + title.encode(), f"search-{title}")
            fixture.poll(lambda: title in client.text() and "Loading" not in client.text(),
                         15, f"Hub did not find {title}")
            for _ in range(5):
                if any("›" in line and title in line for line in client.text().splitlines()):
                    break
                client.send(b"\x1b[B", "select-session-search-result")
                time.sleep(0.15)
            fixture.require(any("›" in line and title in line for line in client.text().splitlines()),
                            f"Hub did not select {title}; screen={client.text()}")
            client.send(b"\r", f"open-{title}")
            fixture.poll(lambda: "└" in client.text() and "Ctrl+N new session" not in client.text(),
                         15, f"Hub did not open {title}")

        process = start_server()
        wid = api.call("POST", "/api/v1/workspaces/resolve",
                       {"path": str(workspace), "create_if_missing": True})[0]["id"]
        source = fixture.create_session(api, wid, "plan-status-source")
        empty = fixture.create_session(api, wid, "plan-status-empty")
        tool(source, "set", {
            "objective": "Persisted plan status probe", "autorun": True,
            "steps": [{"title": "First"}, {"title": "Second"}, {"title": "Third"}],
        })
        tool(source, "edit", {"step": 1, "status": "skipped"})

        # Lose the plugin host's in-memory display contributions while keeping
        # durable plan storage. The first TUI read must recover the chip itself.
        process.terminate()
        process.wait(timeout=10)
        process = start_server()
        displays = api.get("/api/v1/plugins/surface")["catalog"]["terminal"]["display"]
        fixture.require(not any(row["contribution"]["id"] == f"plan:{source}" for row in displays),
                        "cold server unexpectedly retained the in-memory plan contribution")
        client = tui.Pty(binary, api, workspace, source, out / "tui", env)
        progress("⏳ 1/3 ↻", "persisted-plan-visible-after-cold-start")

        # No provider turn or transcript change occurs: plan.changed alone must
        # cause a canonical read and repaint, including skipped-step counting.
        tool(source, "edit", {"step": 2, "status": "completed"})
        progress("⏳ 2/3 ↻", "live-step-update-without-transcript-change")
        tool(source, "phase", {"autorun": False})
        progress("⏳ 2/3", "live-autorun-off")
        fixture.poll(lambda: "↻" not in border(), 10, "old autorun flag remained visible")

        # Exercise the relocated pointer target and the viewer's own canonical
        # summary, then toggle autorun using an actual terminal key.
        row, line = next((index + 1, line) for index, line in enumerate(client.text().splitlines())
                         if "└" in line and "⏳ 2/3" in line)
        prefix = line[:line.index("⏳")]
        col = 1 + sum(2 if unicodedata.east_asian_width(char) in ("W", "F") else 1 for char in prefix)
        client.send(f"\x1b[<0;{col};{row}M\x1b[<0;{col};{row}m".encode(), "click-plan-chip")
        client.wait_text("Persisted plan status probe", "click-opens-plan-document")
        client.send(b"a", "toggle-plan-autorun")
        fixture.poll(lambda: "⏳ 2/3 ↻" in client.text(), 15,
                     "viewer summary did not update after toggling autorun")
        checkpoint("viewer-summary-updates-after-toggle")
        client.send(b"\x1b", "close-plan-viewer")
        progress("⏳ 2/3 ↻", "composer-reflects-viewer-toggle")
        client.resize(24, 42)
        fixture.poll(lambda: border().endswith("┘"), 10, "narrow composer border did not redraw")
        progress("⏳ 2/3 ↻", "plan-chip-survives-narrow-terminal")
        client.resize(36, 120)

        process.terminate()
        process.wait(timeout=10)
        process = start_server()
        tool(source, "edit", {"step": 3, "status": "completed"})
        progress("⏳ 3/3 ↻", "reconnect-reads-plan-after-server-restart")

        switch_to("plan-status-empty")
        fixture.poll(lambda: "3/3" not in border(), 10, "previous session's plan leaked into empty session")
        tool(source, "edit", {"step": 3, "status": "pending"})
        fixture.require("2/3" not in border(), "background session plan leaked into the current session")
        tool(empty, "set", {"objective": "Other plan", "autorun": False, "steps": [{"title": "Only step"}]})
        progress("⏳ 0/1", "idle-session-plan-creation-is-live")
        switch_to("plan-status-source")
        progress("⏳ 2/3 ↻", "session-switch-loads-current-plan")
        tool(source, "clear")
        fixture.poll(lambda: "2/3" not in border() and "↻" not in border(), 15,
                     "clearing the plan left a stale chip")
        checkpoint("clearing-plan-removes-chip")
        result["status"] = "passed"
    except BaseException:
        result["error"] = traceback.format_exc()
    finally:
        if client is not None:
            client.close()
        if process is not None:
            process.terminate()
            try:
                process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        trace.close()
        log.close()
        fixture.write_json(out / "result.json", result)
    print(result)
    return 0 if result["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
