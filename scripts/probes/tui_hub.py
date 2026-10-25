#!/usr/bin/env python3
"""Verify project-scoped Hub navigation in a real, isolated server and PTY.

Uses temporary workspaces and owned instance data; never reads or changes the
installed instance. No provider calls are needed. Saves API/terminal evidence.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import socket
import subprocess
import time
import traceback

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
    process = None
    clients = []
    tui.fixture = fixture
    try:
        alpha = fixture.prepare_configuration(out, "http://127.0.0.1:1/v1")
        beta = out / "project-beta"
        beta.mkdir()
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        env = fixture.clean_environment(out)
        env["AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES"] = "1"
        process = subprocess.Popen(
            [str(binary), "--data-dir", str(out / "data"), "server", "--host", "127.0.0.1",
             "--port", str(port), "--workspace", str(alpha)],
            cwd=out / "process-cwd", env=env, stdout=log, stderr=subprocess.STDOUT)
        api = fixture.Api(f"http://127.0.0.1:{port}", 30, trace)
        fixture.wait_server(process, api, out / "data/run/server.json")

        def resolve(path: Path) -> int:
            return api.call("POST", "/api/v1/workspaces/resolve",
                            {"path": str(path), "create_if_missing": True})[0]["id"]

        aid, bid = resolve(alpha), resolve(beta)
        pins = []
        for index in range(24):
            sid = fixture.create_session(api, aid, f"A-pin-{index:02}")
            api.call("PUT", f"/api/v1/sessions/{sid}", {"pinned": True, "favorite": index == 0})
            pins.append(sid)
        # All pins fall outside the normal 20-session directory page.
        for index in range(22):
            fixture.create_session(api, aid, f"A-recent-{index:02}")
        bsid = fixture.create_session(api, bid, "B-only-marked")
        api.call("PUT", f"/api/v1/sessions/{bsid}", {"pinned": True, "favorite": True})

        def sessions(wid: int) -> list:
            return api.get(f"/api/v1/sessions?workspace_id={wid}&limit=100")["items"]

        def launch(path: Path, label: str) -> tui.Pty:
            client = tui.Pty(binary, api, path, None, out / label, env)
            clients.append(client)
            client.wait_text("Session Hub", "opened")
            return client

        def selected(client: tui.Pty, label: str) -> bool:
            # Inspect only the project viewport; global quick access has its own selection.
            return any(
                line.lstrip("│ ").startswith("> ") and label in line[:tui.COLS // 2]
                for line in client.text().splitlines()
            )

        def project_text(client: tui.Pty) -> str:
            return "\n".join(line[:tui.COLS // 2] for line in client.text().splitlines())

        def wait_selected(client: tui.Pty, label: str) -> None:
            fixture.poll(lambda: selected(client, label), 30, f"selection did not reach {label}: {client.text()}")

        def checkpoint(client: tui.Pty, label: str) -> None:
            (client.directory / f"{label}.txt").write_text(client.text())
            result["cases"].append(label)

        client = launch(alpha, "alpha-create")
        wait_selected(client, "+ New session")
        client.wait_text("A-pin-23", "old-pin-visible")
        left = [line[:tui.COLS // 2] for line in client.text().splitlines()]
        fixture.require(next(i for i, line in enumerate(left) if "workspace" in line)
                        < next(i for i, line in enumerate(left) if "+ New session" in line),
                        "create action appears outside the project")
        checkpoint(client, "initial-project-create-selected")
        before = len(sessions(aid))
        client.send(b"\r", "create-in-launch-project")
        fixture.poll(lambda: len(sessions(aid)) == before + 1, 30, "Enter created no session in launch project")
        fixture.require(len(sessions(bid)) == 1, "Enter created in the wrong project")
        checkpoint(client, "enter-creates-in-launch-project")
        client.close()
        clients.remove(client)

        client = launch(alpha, "project-buckets")
        client.wait_text("A-pin-23", "pins-loaded")
        client.send(b"\t", "select-project-pins")
        wait_selected(client, "A-pin-23")
        client.send(b"\x1b[C", "next-project-pin-page")
        wait_selected(client, "A-pin-03")
        checkpoint(client, "old-pins-second-page")
        # A server-side metadata change causes a real catalog refresh.
        # Rename another pin: this reorders the page while the selected pin
        # survives, so preserving a raw row index would focus the wrong session.
        api.call("PUT", f"/api/v1/sessions/{pins[0]}", {"title": "A-pin-00-renamed"})
        client.wait_text("A-pin-00-renamed", "live-refresh")
        wait_selected(client, "A-pin-03")
        checkpoint(client, "refresh-keeps-project-and-page")
        client.send(b"f", "favorite-old-pin")
        fixture.poll(lambda: api.state(pins[3])["session"]["favorite"], 30, "TUI favorite toggle failed")
        client.wait_text("Favorites  2", "favorite-bucket-updated")
        client.send(b"\t", "select-project-favorites")
        wait_selected(client, "A-pin-03")
        client.send(b"p", "unpin-while-keeping-favorite")
        fixture.poll(lambda: not api.state(pins[3])["session"]["pinned"], 30, "TUI unpin failed")
        fixture.require(api.state(pins[3])["session"]["favorite"], "unpin cleared favorite")
        checkpoint(client, "pin-favorite-independent")
        client.send(b"\x1b[H", "select-first-project")
        client.send(b"\r", "collapse-first-project")
        client.wait_text("project-beta", "beta-directory-visible")
        time.sleep(0.2)
        client.send(b"\x1b[B", "select-second-project")
        client.send(b"\r", "expand-second-project")
        fixture.poll(lambda: "B-only-marked" in project_text(client), 30, "second project's buckets did not load")
        left = project_text(client)
        fixture.require("A-pin-" not in left, "collapsed first project's pins leaked into second project")
        fixture.require("Pinned" in left and "Favorites" in left, "second project has no marked buckets")
        checkpoint(client, "each-project-has-own-buckets")
        client.send(b"\x1b[B", "select-beta-create")
        wait_selected(client, "+ New session")
        client.send(b"\r", "create-in-second-project")
        fixture.poll(lambda: len(sessions(bid)) == 2, 30, "Enter did not create in selected project")
        checkpoint(client, "enter-creates-in-selected-project")
        client.close()
        clients.remove(client)

        # Starting directly in another folder must use that folder as the default.
        client = launch(beta, "beta-launch")
        wait_selected(client, "+ New session")
        fixture.poll(lambda: "B-only-marked" in project_text(client), 30, "launch project's buckets did not load")
        checkpoint(client, "launch-folder-determines-default-project")
        result["status"] = "passed"
    except BaseException:
        result["error"] = traceback.format_exc()
    finally:
        for client in clients:
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
