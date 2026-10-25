#!/usr/bin/env python3
"""Verify height-aware Hub paging and live resizing in an isolated real PTY."""
from __future__ import annotations

import argparse
from pathlib import Path
import re
import socket
import subprocess
import threading
import time
import traceback
from urllib.parse import parse_qs, urlparse

import http_experience as fixture
import tui_experience as tui
from tui_session_marks import DelayedProxy


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
    process = proxy = client = None
    tui.fixture = fixture
    tui.ROWS, tui.COLS = 24, 100
    try:
        workspace = fixture.prepare_configuration(out, "http://127.0.0.1:1/v1")
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        env = fixture.clean_environment(out)
        env["AGENA_DISABLE_PUBLIC_MODEL_CATALOG_SOURCES"] = "1"
        process = subprocess.Popen(
            [str(binary), "--data-dir", str(out / "data"), "server", "--host", "127.0.0.1",
             "--port", str(port), "--workspace", str(workspace)],
            cwd=out / "process-cwd", env=env, stdout=log, stderr=subprocess.STDOUT)
        api = fixture.Api(f"http://127.0.0.1:{port}", 30, trace)
        fixture.wait_server(process, api, out / "data/run/server.json")
        wid = api.call("POST", "/api/v1/workspaces/resolve",
                       {"path": str(workspace), "create_if_missing": True})[0]["id"]
        ids = [fixture.create_session(api, wid, f"Page-item-{index:03}") for index in range(120)]
        proxy = DelayedProxy(port, trace)
        threading.Thread(target=proxy.serve_forever, daemon=True).start()
        proxied_api = fixture.Api(f"http://127.0.0.1:{proxy.server_port}", 30, trace)
        client = tui.Pty(binary, proxied_api, workspace, None, out / "tui", env)

        def reads(after=0):
            with proxy.lock:
                requests = list(proxy.requests[after:])
            return [parse_qs(urlparse(path).query) for method, path in requests
                    if method == "GET" and urlparse(path).path == "/api/v1/sessions"]

        def requested(limit, offset, after=0):
            return any(params.get("limit") == [str(limit)] and
                       params.get("offset", ["0"]) == [str(offset)] and
                       not params.get("bucket") for params in reads(after))

        def selected():
            return next((match.group() for line in client.text().splitlines()
                         if line.lstrip("│ ").startswith("›")
                         and (match := re.search(r"Page-item-\d{3}", line))), None)

        def checkpoint(label):
            fixture.require(not client.failure, str(client.failure))
            (client.directory / f"{label}.txt").write_text(client.text())
            result["cases"].append(label)

        def loaded(limit, offset=0, after=0, selection=None):
            fixture.poll(lambda: requested(limit, offset, after) and "Loading" not in client.text()
                         and f"page {offset // limit + 1}" in client.text()
                         and (selection is None or selected() == selection),
                         30, f"page did not settle: limit={limit}, offset={offset}, screen={client.text()}")

        def check_first_page(rows, count):
            client.resize(rows)
            loaded(count)
            def labels():
                return [re.search(r"Page-item-\d{3}", line).group()
                        for line in client.text().splitlines()
                        if "Idle" in line and re.search(r"Page-item-\d{3}", line)]
            expected = [f"Page-item-{i:03}" for i in range(119, 119 - count, -1)]
            fixture.poll(lambda: labels() == expected, 10,
                         f"wrong rendered capacity for {rows} rows: {labels()}")
            fixture.require("Next page" in client.text(), "paging control did not fit the terminal")
            lines = client.text().splitlines()
            name_row = next(i for i, line in enumerate(lines) if "Name" in line and "Status" in line)
            fixture.require(name_row == 2, "top action/help rows still occupy the Hub header")
            footer = "\n".join(lines[-3:])
            for hint in ("↑↓ move", "Enter open / expand", "Tab/⇧Tab view", "[ ] page", "PgUp/Dn scroll",
                         "Ctrl+N new session", "/ search", "p pin", "f favorite", "Ctrl+R refresh"):
                fixture.require(hint in footer, f"missing unified footer action: {hint}")
            checkpoint(f"height-{rows}-shows-{count}-sessions")

        client.wait_text("Page-item-119", "opened")
        check_first_page(24, 11)
        check_first_page(36, 23)
        check_first_page(60, 47)
        check_first_page(36, 23)
        # Select a non-first item on a later page: a resize must use its
        # absolute position, not just the page's old starting offset.
        client.send(b"\x1b[B", "select-first-session")
        fixture.poll(lambda: selected() == "Page-item-119", 10, "first session not selected")
        for page in (1, 2):
            client.send(b"]", "next-page")
            loaded(23, page * 23, selection=f"Page-item-{119 - page * 23:03}")
        client.send(b"\x1b[B" * 7, "select-inside-later-page")
        anchor = "Page-item-066"
        fixture.poll(lambda: selected() == anchor, 10, "anchor not selected")
        client.resize(60)
        loaded(47, 47, selection=anchor)
        checkpoint("grow-later-page-keeps-selected-session")
        client.resize(24)
        loaded(11, 44, selection=anchor)
        checkpoint("shrink-later-page-keeps-selected-session")

        # All old size responses are held, then released together. The final
        # size must win while the still-loaded selected session stays visible.
        proxy.reads.clear()
        proxy.captured_read.clear()
        with proxy.lock:
            after = len(proxy.requests)
        client.resize(48)
        fixture.require(proxy.captured_read.wait(5), "resize request was not held")
        for rows, limit, offset in [(20, 7, 49), (60, 47, 47), (28, 15, 45)]:
            client.resize(rows)
            fixture.poll(lambda: requested(limit, offset, after), 5, "resize query did not use anchor")
            fixture.require(selected() == anchor, "loading a resize cleared the selected session")
        proxy.reads.set()
        loaded(15, 45, after, anchor)
        time.sleep(0.5)
        fixture.require(selected() == anchor and "page 4" in client.text(),
                        "late size response overwrote the final page")
        checkpoint("rapid-resizes-ignore-stale-catalogs")

        # The search field replaces the create action without changing capacity.
        client.send(b"/Page-item", "search-with-adaptive-pages")
        loaded(15)
        fixture.require("+ New session" not in client.text(), "search retained create row")
        checkpoint("search-keeps-footer-and-page-geometry-consistent")
        client.send(b"\x1b", "close-search")
        loaded(15)
        # A whitespace filter still has a visible input field after moving
        # selection out of the editor. Its height must remain accounted for.
        client.send(b"/   ", "search-whitespace")
        loaded(14)
        client.send(b"\x1b[B", "leave-whitespace-search-editor")
        time.sleep(0.4)
        normal_reads = [params for params in reads() if not params.get("bucket")]
        fixture.require(normal_reads[-1].get("limit") == ["14"] and "Filter:" in client.text(),
                        "leaving a whitespace filter changed page geometry")
        checkpoint("whitespace-search-keeps-visible-input-height")
        client.send(b"\x1b", "clear-whitespace-search")
        loaded(15)
        # Pinned/Favorites must also use the terminal-sized limit, not 20.
        for sid in ids[:30]:
            api.call("PUT", f"/api/v1/sessions/{sid}", {"pinned": True, "favorite": True})
        client.send(b"\x12", "refresh-marked-buckets")
        client.wait_text("Pinned  30", "pins-refreshed")
        fixture.poll(lambda: "Favorites  30" in client.text() or
                     any(params.get("bucket") == ["favorite"] and params.get("limit") == ["15"]
                         for params in reads()), 10, "favorites did not use adaptive page size")
        bucket_reads = reads()
        for bucket in ("pinned", "favorite"):
            fixture.require(any(params.get("bucket") == [bucket] and params.get("limit") == ["15"]
                                for params in bucket_reads), f"{bucket} still uses a fixed limit")
        checkpoint("project-mark-buckets-use-adaptive-pages")
        client.send(b"\t", "switch-to-project-tree")
        fixture.poll(lambda: "h/l ←→ project" in client.text() and "Loading" not in client.text(),
                     30, "project tree did not settle")
        client.send(b"\t", "switch-to-global-sessions")
        fixture.poll(lambda: "▾" not in client.text() and "Loading" not in client.text(),
                     30, "global sessions did not settle")
        checkpoint("other-views-keep-independent-page-sizes")
        result["status"] = "passed"
    except BaseException:
        result["error"] = traceback.format_exc()
    finally:
        if proxy is not None:
            proxy.reads.set()
            proxy.writes.set()
        if client is not None:
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
