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
import threading
from urllib.parse import parse_qs, urlparse

import http_experience as fixture
import tui_experience as tui
from tui_session_marks import DelayedProxy


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
    process = proxy = None
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

        proxy = DelayedProxy(port, trace)
        threading.Thread(target=proxy.serve_forever, daemon=True).start()
        proxied_api = fixture.Api(f"http://127.0.0.1:{proxy.server_port}", 30, trace)

        def sessions(wid: int) -> list:
            return api.get(f"/api/v1/sessions?workspace_id={wid}&limit=100")["items"]

        def launch(path: Path, label: str) -> tui.Pty:
            client = tui.Pty(binary, proxied_api, path, None, out / label, env)
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
            # Every view now uses the entire content rectangle.
            return client.text()

        def wait_scope(client: tui.Pty, view: str) -> None:
            def ready():
                text = client.text()
                if "Loading" in text:
                    return False
                if view == "Projects":
                    return "h/l ←→ project" in text
                if view == "All sessions":
                    return ("All sessions  " in text or "Pinned  " in text) and "▾" not in text
                return "h/l ←→ project" not in text and "▾ workspace" in text
            fixture.poll(ready, 30, f"view did not become {view}")

        def click_view(client: tui.Pty, label: str) -> None:
            row, line = next((index + 1, line) for index, line in enumerate(client.text().splitlines())
                             if "Current project" in line and "All sessions" in line)
            col = line.index(label) + 2
            client.send(f"\x1b[<0;{col};{row}M\x1b[<0;{col};{row}m".encode(), f"click-view-{label}")

        def select_bucket_item(client: tui.Pty, label: str, bucket: str) -> None:
            client.send(b"\x1b[H", "start-bucket-navigation")
            for index in range(100):
                time.sleep(0.06)
                lines = client.text().splitlines()
                selected_index = next((i for i, line in enumerate(lines)
                                       if line.lstrip("│ ").startswith("›") and label in line), None)
                if selected_index is not None:
                    header = next((line for line in reversed(lines[:selected_index])
                                   if any(kind + "  " in line for kind in ("Pinned", "Favorites", "Recent", "All sessions"))), "")
                    if bucket + "  " in header:
                        return
                client.send(b"\x1b[B", f"find-{bucket}-{index}")
            raise AssertionError(f"Could not select {label} in {bucket}: {client.text()}")

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

        fixture.require("project-beta" not in client.text() and "B-only-marked" not in client.text(),
                        "current-project view leaked another project")
        with proxy.lock:
            reads = [urlparse(path) for method, path in proxy.requests if method == "GET"]
        fixture.require(not any(url.path == "/api/v1/workspaces" for url in reads),
                        "current project unnecessarily enumerated other workspaces")
        session_reads = [parse_qs(url.query) for url in reads if url.path == "/api/v1/sessions"]
        fixture.require(session_reads and all(params.get("workspace_id") == [str(aid)] for params in session_reads),
                        "current-project API reads escaped the launch workspace")
        checkpoint(client, "startup-current-project-scope")
        client.send(b"l\x1b[C", "horizontal-noop-in-current-project")
        time.sleep(0.2)
        wait_selected(client, "+ New session")
        fixture.require("Resume Session" not in client.text(), "l still opens full list")
        client.send(b"\t", "switch-to-projects")
        client.wait_text("h/l ←→ project", "projects-visible")
        fixture.require(not any("Running " in line and "Recent " in line for line in client.text().splitlines()),
                        "project tree still has global activity summaries")
        fixture.poll(lambda: "Loading" not in client.text(), 30, "project catalog did not finish")
        checkpoint(client, "tab-selects-project-tree-without-global-summaries")
        for forward, backward in [(b"l", b"h"), (b"\x1b[C", b"\x1b[D")]:
            client.send(forward, "next-project")
            client.wait_text("B-only-marked", "beta-expanded")
            wait_selected(client, "+ New session")
            lines = client.text().splitlines()
            index = next(i for i, line in enumerate(lines) if line.lstrip("│ ").startswith("›"))
            fixture.require(any("▾ project-beta" in line for line in lines[:index]),
                            "project destination was lost")
            client.send(backward, "previous-project")
            wait_selected(client, "+ New session")
            time.sleep(0.2)
        checkpoint(client, "h-l-and-arrows-switch-projects")
        client.send(b"\t", "switch-to-all-sessions")
        wait_scope(client, "All sessions")
        client.wait_text("Pinned  25", "global-pins-loaded")
        client.wait_text("Favorites  2", "global-favorites-loaded")
        fixture.require("A-pin-00" in client.text() and "B-only-marked" in client.text(),
                        "global marks did not include older sessions and both projects")
        fixture.require("+ New session" not in client.text() and "▾" not in client.text() and "▸" not in client.text(),
                        "flat list still groups by project")
        checkpoint(client, "all-sessions-has-cross-project-pins-and-favorites")
        select_bucket_item(client, "B-only-marked", "Pinned")
        client.send(b"]", "global-pins-next-page")
        wait_selected(client, "A-pin-04")
        checkpoint(client, "global-pin-pagination-includes-old-sessions")
        client.send(b"[", "global-pins-previous-page")
        wait_selected(client, "A-pin-05")
        select_bucket_item(client, "B-only-marked", "Favorites")
        client.send(b"f", "unfavorite-global-item")
        fixture.poll(lambda: not api.state(bsid)["session"]["favorite"], 30, "global unfavorite failed")
        client.wait_text("Favorites  1", "global-favorite-count-updated")
        fixture.require(api.state(bsid)["session"]["pinned"], "global unfavorite cleared the pin")
        wait_selected(client, "B-only-marked")
        client.send(b"f", "restore-global-favorite")
        fixture.poll(lambda: api.state(bsid)["session"]["favorite"], 30, "global favorite restore failed")
        client.wait_text("Favorites  2", "global-favorite-restored")
        checkpoint(client, "global-marks-update-counts-and-preserve-selection")
        select_bucket_item(client, "B-only-marked", "All sessions")
        client.send(b"\x1b[B", "show-flat-sessions-from-another-project")
        wait_selected(client, "A-recent-21")
        fixture.require("▾" not in client.text(), "all sessions reintroduced project grouping")
        client.send(b"\x1b[A", "select-first-flat-session")
        wait_selected(client, "B-only-marked")
        checkpoint(client, "all-sessions-flat-cross-project-view")
        client.send(b"]", "flat-next-page")
        wait_selected(client, "A-recent-02")
        client.send(b"[", "flat-previous-page")
        wait_selected(client, "A-recent-03")
        checkpoint(client, "flat-pagination-independent-of-global-marks")
        client.send(b"\t", "wrap-to-current-project")
        client.wait_text("A-pin-23", "current-restored")
        fixture.require("B-only-marked" not in client.text(), "view wrap retained foreign sessions")
        client.send(b"/B-only-marked", "search-current-project-only")
        client.wait_text("Filter: B-only-marked", "current-search-started")
        time.sleep(0.6)
        fixture.require(not any("B-only-marked" in line and "Idle" in line for line in client.text().splitlines()),
                        "current-project search escaped its scope")
        client.send(b"\t", "keep-search-in-projects")
        fixture.poll(lambda: any("B-only-marked" in line and "Idle" in line
                                 for line in client.text().splitlines()), 30, "grouped search did not load")
        client.wait_text("project-beta", "grouped-search-project")
        checkpoint(client, "project-search-groups-cross-project-results")
        client.send(b"\t", "keep-search-in-all-sessions")
        fixture.poll(lambda: any("B-only-marked" in line and "Idle" in line for line in client.text().splitlines())
                     and "▾" not in client.text(), 30, "flat search did not load")
        client.wait_text("Pinned  1", "global-pin-search")
        client.wait_text("Favorites  1", "global-favorite-search")
        fixture.require("A-pin-" not in client.text(), "global mark search leaked a nonmatching session")
        checkpoint(client, "search-survives-view-changes-with-correct-scope")
        client.send(b"\x1b", "close-search-before-mouse-views")
        fixture.poll(lambda: "Filter:" not in client.text() and "Loading" not in client.text(),
                     30, "search did not finish closing")
        click_view(client, "Projects")
        client.wait_text("h/l ←→ project", "mouse-projects")
        client.send(b"\x1b[Z", "shift-tab-to-current")
        client.wait_text("A-pin-23", "shift-tab-current")
        fixture.require("project-beta" not in client.text(), "Shift+Tab did not restore current project")
        click_view(client, "All sessions")
        client.wait_text("B-only-marked", "mouse-flat")
        client.send(b"\x1b[Z", "reverse-to-projects")
        client.wait_text("h/l ←→ project", "reverse-projects")
        client.send(b"\x1b[Z", "reverse-to-current")
        client.wait_text("A-pin-23", "reverse-current")
        checkpoint(client, "mouse-tabs-and-reverse-tab-cycle")

        client.send("aqXZ09中".encode(), "ordinary-text-with-search-closed")
        time.sleep(0.4)
        fixture.require("Filter:" not in client.text() and "A-pin-23" in client.text(),
                        "ordinary text opened or filtered Hub search")
        wait_selected(client, "+ New session")
        checkpoint(client, "ordinary-text-does-not-open-search")

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
        client.send("aqXZ09中".encode(), "ordinary-text-after-closing-search")
        time.sleep(0.4)
        fixture.require("Filter:" not in client.text(), "ordinary text reopened canceled search")
        checkpoint(client, "ordinary-text-does-not-reopen-canceled-search")
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
        client.send(b"\x1b[B", "select-project-pins")
        wait_selected(client, "A-pin-23")
        client.send(b"]", "next-project-pin-page")
        wait_selected(client, "A-pin-03")
        checkpoint(client, "old-pins-second-page")
        project = project_text(client)
        fixture.require("Name" in project and "Status" in project and "Updated" in project,
                        "Hub has no explicit name/status/time headings")
        selected_line = next(line for line in project.splitlines() if "›" in line and "A-pin-03" in line)
        fixture.require("Idle" in selected_line and selected_line.count("│") >= 3
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
        select_bucket_item(client, "A-pin-03", "Favorites")
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
        client.send(b"\t", "switch-projects-for-create")
        client.wait_text("h/l ←→ project", "projects-ready-for-create")
        client.send(b"\x1b[H", "select-first-project")
        client.send(b"\r", "collapse-first-project")
        client.wait_text("project-beta", "beta-directory-visible")
        time.sleep(0.2)
        client.send(b"l", "select-and-expand-second-project")
        fixture.poll(lambda: "B-only-marked" in project_text(client), 30, "second project's buckets did not load")
        left = project_text(client)
        fixture.require("A-pin-" not in left, "collapsed first project's pins leaked into second project")
        fixture.require("Pinned" in left and "Favorites" in left, "second project has no marked buckets")
        checkpoint(client, "each-project-has-own-buckets")
        # Project navigation lands on that project's create action.
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
        client.close()
        clients.remove(client)
        # Hold old catalog snapshots while changing scopes repeatedly. All
        # responses resume together; only the final view may populate rows.
        proxy.reads.clear()
        proxy.captured_read.clear()
        client = launch(alpha, "delayed-switches")
        fixture.require(proxy.captured_read.wait(5), "catalog read was not held")
        for packet in [b"\t", b"\t", b"\x1b[Z", b"\x1b[Z", b"\x1b[Z"]:
            client.send(packet, "switch-with-delayed-reads")
            time.sleep(0.12)
        proxy.reads.set()
        wait_scope(client, "All sessions")
        client.wait_text("B-only-marked", "delayed-final-flat-view")
        time.sleep(0.8)
        fixture.require("▾" not in client.text() and "+ New session" not in client.text(),
                        "old project response overwrote the final flat view")
        checkpoint(client, "late-catalogs-cannot-overwrite-current-view")
        client.send(b"\t", "return-to-current-after-late-replies")
        wait_scope(client, "Current project")
        client.wait_text("A-pin-", "current-after-late-replies")
        fixture.require("project-beta" not in client.text() and "B-only-marked" not in client.text(),
                        "foreign rows survived final current-project selection")
        checkpoint(client, "current-project-scope-survives-delayed-view-switches")
        result["status"] = "passed"
    except BaseException:
        result["error"] = traceback.format_exc()
    finally:
        if proxy is not None:
            proxy.reads.set()
            proxy.writes.set()
        for client in clients:
            client.close()
        if proxy is not None:
            proxy.shutdown()
            proxy.server_close()
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
