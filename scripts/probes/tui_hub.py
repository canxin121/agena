#!/usr/bin/env python3
"""Verify project-scoped Hub navigation in a real, isolated server and PTY.

Uses temporary workspaces and owned instance data; never reads or changes the
installed instance. No provider calls are needed. Saves API/terminal evidence.
"""
from __future__ import annotations

import argparse
from pathlib import Path
import re
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
    parser.add_argument("--cols", type=int, default=120)
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=False)
    trace = fixture.JsonLines(out / "http.jsonl")
    log = (out / "server.log").open("wb")
    result = {"status": "failed", "binary_sha256": fixture.file_digest(binary), "cols": args.cols, "cases": []}
    process = None
    clients = []
    tui.fixture = fixture
    tui.COLS = args.cols
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
                line.lstrip("│ ").startswith("›") and label in line
                for line in project_text(client).splitlines()
            )

        def project_text(client: tui.Pty) -> str:
            lines = client.text().splitlines()
            if tui.COLS - 2 >= 196:
                return "\n".join(line[:1 + (tui.COLS - 2) // 2] for line in lines)
            divider = next((index for index, line in enumerate(lines)
                            if "Running " in line and "Recent " in line), len(lines))
            return "\n".join(lines[:divider])

        def wait_selected(client: tui.Pty, label: str) -> None:
            fixture.poll(lambda: selected(client, label), 30, f"selection did not reach {label}: {client.text()}")

        def checkpoint(client: tui.Pty, label: str) -> None:
            (client.directory / f"{label}.txt").write_text(client.text())
            result["cases"].append(label)

        client = launch(alpha, "alpha-create")
        wait_selected(client, "+ New session")
        # Root Escape must neither reveal the empty conversation route nor
        # cancel the catalog request that may still be loading.
        client.send(b"\x1b", "escape-on-startup-hub")
        time.sleep(0.2)
        fixture.require("Session Hub" in client.text(), "Escape dismissed the startup Hub")
        client.wait_text("A-pin-23", "old-pin-visible")
        for _ in range(2):
            client.send(b"\x1b", "escape-on-loaded-hub")
            time.sleep(0.15)
            fixture.require("Session Hub" in client.text(), "repeated Escape dismissed the Hub")
            wait_selected(client, "+ New session")
        fixture.require(len(sessions(aid)) == 46, "Escape unexpectedly created a session")
        checkpoint(client, "root-escape-preserves-hub-selection-and-loading")
        fixture.require("Esc back" not in client.text(), "root Hub still advertises Escape back")

        client.send(b"\x06", "open-empty-hub-search")
        client.wait_text("Filter:", "empty-search-active")
        client.send(b"\x1b", "cancel-empty-hub-search")
        time.sleep(0.2)
        fixture.require("Session Hub" in client.text() and "Filter:" not in client.text(),
                        "Escape did not cancel empty search inside the Hub")
        checkpoint(client, "escape-cancels-empty-search-without-leaving-hub")
        client.send(b"/A-pin-23", "filter-hub-sessions")
        client.wait_text("Filter: A-pin-23", "search-query-active")
        client.send(b"\x1b", "clear-hub-search")
        time.sleep(0.2)
        fixture.require("Session Hub" in client.text() and "Filter:" not in client.text(),
                        "Escape did not clear search inside the Hub")
        client.send(b"\x1b", "escape-after-clearing-search")
        time.sleep(0.2)
        fixture.require("Session Hub" in client.text(), "Escape after clearing search dismissed the Hub")
        checkpoint(client, "escape-clears-search-and-keeps-hub-as-root")
        client.send(b"\x1b[H", "select-launch-project-after-search")
        client.wait_text("Pinned  24", "catalog-restored-after-search")
        client.send(b"\x1b[B", "select-new-session-after-search")
        wait_selected(client, "+ New session")
        left = project_text(client).splitlines()
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
        project = project_text(client)
        fixture.require("Name" in project and "Status" in project and "Updated" in project,
                        "Hub has no explicit name/status/time headings")
        selected_line = next(line for line in project.splitlines() if "›" in line and "A-pin-03" in line)
        fixture.require("Ready" in selected_line and selected_line.count("│") >= 3
                        and re.search(r"\d{2}-\d{2} \d{2}:\d{2}", selected_line),
                        "session metadata is missing or not in separate columns")
        fixture.require(selected_line.index("A-pin-03") - selected_line.index("›") == 2,
                        "session name still has excessive leading padding")
        checkpoint(client, "explicit-name-status-time-columns")
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
        # Updating metadata can move this session out of page 2 of Pins.
        # Selection follows its identity into Recent, so navigate to Favorites
        # explicitly instead of assuming the old bucket is still selected.
        client.send(b"\x1b[H\t\t", "select-project-favorites")
        def favorite_selected() -> bool:
            lines = project_text(client).splitlines()
            index = next((i for i, line in enumerate(lines)
                          if "›" in line and "A-pin-03" in line), None)
            if index is None:
                return False
            header = next((line for line in reversed(lines[:index])
                           if any(label in line for label in ("Pinned  ", "Favorites  ", "Recent  "))), "")
            return "Favorites  " in header
        fixture.poll(favorite_selected, 30, "target was not selected in Favorites")
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
