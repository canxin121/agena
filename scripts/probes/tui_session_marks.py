#!/usr/bin/env python3
"""Exercise rapid Hub marks with delayed HTTP replies in an isolated real PTY."""
from __future__ import annotations

import argparse
import http.client
import http.server
import json
from pathlib import Path
import re
import socket
import subprocess
import threading
import time
import traceback

import http_experience as fixture
import tui_experience as tui


class DelayedProxy(http.server.ThreadingHTTPServer):
    daemon_threads = True

    def __init__(self, port: int, trace):
        super().__init__(("127.0.0.1", 0), ProxyHandler)
        self.upstream_port = port
        self.trace = trace
        self.writes = threading.Event()
        self.writes.set()
        self.reads = threading.Event()
        self.reads.set()
        self.captured_read = threading.Event()
        self.write_count = 0
        self.fail_next = False
        self.lock = threading.Lock()
        self.requests = []


class ProxyHandler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def forward(self):
        proxy = self.server
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        with proxy.lock:
            proxy.requests.append((self.command, self.path))
        is_write = self.command == "PUT" and re.fullmatch(r"/api/v1/sessions/\d+", self.path)
        fail = False
        if is_write:
            with proxy.lock:
                proxy.write_count += 1
                fail, proxy.fail_next = proxy.fail_next, False
        connection = http.client.HTTPConnection("127.0.0.1", proxy.upstream_port, timeout=30)
        try:
            if fail:
                status = 500
                headers = [("Content-Type", "application/json")]
                payload = json.dumps({"error": {"code": "internal", "message": "injected marker failure"}}).encode()
            else:
                headers_in = {k: v for k, v in self.headers.items()
                              if k.lower() not in ("host", "connection", "content-length")}
                connection.request(self.command, self.path, body or None, headers_in)
                response = connection.getresponse()
                status, headers = response.status, response.getheaders()
                if "text/event-stream" in response.getheader("Content-Type", ""):
                    self.send_response(status)
                    for key, value in headers:
                        if key.lower() not in ("transfer-encoding", "connection", "content-length"):
                            self.send_header(key, value)
                    self.end_headers()
                    while data := response.read1(65536):
                        self.wfile.write(data)
                        self.wfile.flush()
                    return
                payload = response.read()
            if is_write:
                proxy.writes.wait(15)
            if self.command == "GET" and self.path.startswith("/api/v1/sessions?") and not proxy.reads.is_set():
                proxy.captured_read.set()
                proxy.reads.wait(15)
            self.send_response(status)
            for key, value in headers:
                if key.lower() not in ("transfer-encoding", "connection", "content-length"):
                    self.send_header(key, value)
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
            proxy.trace.add({"kind": "proxy", "method": self.command, "path": self.path,
                             "request": json.loads(body) if body else None,
                             "response": json.loads(payload) if payload else None, "status": status})
        except (OSError, http.client.HTTPException):
            pass
        finally:
            connection.close()

    do_GET = do_POST = do_PUT = do_DELETE = forward


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
    process = client = proxy = None
    tui.fixture = fixture
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
        neighbor = fixture.create_session(api, wid, "untouched-neighbor")
        api.call("PUT", f"/api/v1/sessions/{neighbor}", {"pinned": True, "favorite": True})
        sid = fixture.create_session(api, wid, "rapid-mark-target")
        proxy = DelayedProxy(port, trace)
        threading.Thread(target=proxy.serve_forever, daemon=True).start()
        proxied_api = fixture.Api(f"http://127.0.0.1:{proxy.server_port}", 30, trace)
        client = tui.Pty(binary, proxied_api, workspace, None, out / "tui", env)
        client.wait_text("rapid-mark-target", "opened")
        # New session -> neighbor in Pins -> neighbor in Favorites -> target in Recent.
        client.send(b"\x1b[B\x1b[B\x1b[B", "select-target-in-project-recent")

        def selected_line():
            return next((line for line in client.text().splitlines()
                         if "›" in line and "rapid-mark-target" in line), "")

        fixture.poll(selected_line, 10, "target was not selected")

        def visible(pin, favorite):
            line = selected_line()
            return bool(line) and ("◆" in line) == pin and ("★" in line) == favorite

        def settled(pin, favorite):
            state = api.state(sid)["session"]
            return state["pinned"] == pin and state["favorite"] == favorite and visible(pin, favorite)

        def checkpoint(label):
            (client.directory / f"{label}.txt").write_text(client.text())
            result["cases"].append(label)

        def click(mark):
            lines = client.text().splitlines()
            row, line = next((index + 1, line) for index, line in enumerate(lines)
                             if re.search(rf"\b{mark} (?:un)?(?:pin|favorite)", line))
            match = re.search(rf"\b{mark} (?:un)?(?:pin|favorite)", line)
            # An interior character remains in the button for both labels.
            column = match.start() + (3 if mark == "p" else 7) + 1
            return f"\x1b[<0;{column};{row}M\x1b[<0;{column};{row}m".encode()

        current = (False, False)
        for mode in ("keyboard", "mouse"):
            for count in (20, 21):
                for marks in ("p", "f", "pf"):
                    proxy.writes.clear()
                    before = proxy.write_count
                    desired = (current[0] ^ ("p" in marks and count % 2 == 1),
                               current[1] ^ ("f" in marks and count % 2 == 1))
                    packets = {mark: mark.encode() if mode == "keyboard" else click(mark) for mark in marks}
                    client.send(b"".join(packets[mark] for _ in range(count) for mark in marks),
                                f"{mode}-{count}-{marks}")
                    fixture.poll(lambda: proxy.write_count > before, 5, "no metadata write arrived")
                    fixture.poll(lambda: visible(*desired), 5, "latest click was not reflected immediately")
                    # Responses stay blocked while we sample multiple render frames.
                    for _ in range(5):
                        time.sleep(0.1)
                        fixture.require(visible(*desired), "marker or selection flickered while replies were pending")
                    fixture.require(proxy.write_count - before == len(marks), "writes for one marker ran concurrently")
                    proxy.writes.set()
                    fixture.poll(lambda: settled(*desired), 10, "server and display did not converge")
                    time.sleep(0.7)
                    fixture.require(settled(*desired), "catalog refresh reverted a committed marker")
                    fixture.require(proxy.write_count - before <= 2 * len(marks), "rapid inputs were not coalesced")
                    fixture.require(all(api.state(neighbor)["session"][key] for key in ("pinned", "favorite")),
                                    "rapid actions changed the neighboring session")
                    current = desired
                    checkpoint(f"{mode}-{count}-{marks}-consistent")

        # Let an old catalog snapshot finish only after another toggle commits.
        proxy.reads.clear()
        proxy.captured_read.clear()
        client.send(b"\x12", "start-delayed-catalog")
        fixture.require(proxy.captured_read.wait(5), "catalog was not captured")
        client.send(b"p", "toggle-with-old-catalog-pending")
        current = (not current[0], current[1])
        fixture.poll(lambda: settled(*current), 5, "toggle did not commit during old catalog read")
        proxy.reads.set()
        time.sleep(1)
        fixture.require(settled(*current), "late catalog reverted the latest marker")
        checkpoint("stale-catalog-cannot-revert-commit")

        proxy.fail_next = True
        before = proxy.write_count
        client.send(b"f", "inject-failed-favorite")
        fixture.poll(lambda: proxy.write_count > before, 5, "failure injection saw no write")
        fixture.poll(lambda: settled(*current), 10, "failed write did not roll back")
        checkpoint("failure-rolls-back-only-favorite")
        client.send(b"f", "retry-favorite")
        current = (current[0], not current[1])
        fixture.poll(lambda: settled(*current), 10, "retry did not converge")
        checkpoint("retry-after-failure-converges")
        client.close()
        client = tui.Pty(binary, proxied_api, workspace, None, out / "reopened", env)
        client.wait_text("rapid-mark-target", "reopened")
        client.send(b"\x1b[B", "select-persisted-target-in-pins")
        fixture.poll(lambda: settled(*current), 10, "reopening lost the committed markers")
        checkpoint("reopening-preserves-server-state")
        client.send(b"\t\t", "switch-to-flat-view-for-mark-races")
        fixture.poll(lambda: "▾" not in client.text() and visible(*current), 10,
                     "flat view did not select the persisted session")
        for mode in ("keyboard", "mouse"):
            proxy.writes.clear()
            before = proxy.write_count
            desired = tuple(not value for value in current)
            packet = b"pf" if mode == "keyboard" else click("p") + click("f")
            client.send(packet * 21, f"flat-{mode}-rapid-marks")
            fixture.poll(lambda: proxy.write_count - before == 2, 5, "flat view did not submit both marks")
            fixture.poll(lambda: visible(*desired), 5, "flat view did not show the latest intent")
            # Cycle through both other scopes with replies still pending.
            client.send(b"\t\t\t", f"flat-{mode}-switch-while-marks-pending")
            time.sleep(0.3)
            proxy.writes.set()
            fixture.poll(lambda: settled(*desired), 10, "view switches lost pending marks")
            fixture.require("▾" not in client.text(), "pending mark response restored the wrong view")
            fixture.require(all(api.state(neighbor)["session"][key] for key in ("pinned", "favorite")),
                            "view switching changed the neighboring session")
            current = desired
            checkpoint(f"flat-{mode}-marks-survive-view-switches")
        client.send(b"\r", "open-selected-session")
        time.sleep(0.5)
        client.send_prompt("/sessions")
        client.wait_text("Resume Session", "picker-opened")
        fixture.poll(lambda: visible(*current), 10, "picker did not select the latest session")
        for mode in ("keyboard", "mouse"):
            for count in (20, 21):
                proxy.writes.clear()
                before = proxy.write_count
                desired = tuple(value ^ (count % 2 == 1) for value in current)
                packet = b"pf" if mode == "keyboard" else click("p") + click("f")
                client.send(packet * count, f"picker-{mode}-{count}-mixed")
                fixture.poll(lambda: proxy.write_count - before == 2, 5, "picker did not submit both marks")
                fixture.poll(lambda: visible(*desired), 5, "picker lost rapid inputs")
                for _ in range(5):
                    time.sleep(0.1)
                    fixture.require(visible(*desired), "picker marker flickered during pending writes")
                proxy.writes.set()
                fixture.poll(lambda: settled(*desired), 10, "picker and server did not converge")
                time.sleep(0.4)
                fixture.require(proxy.write_count - before <= 4, "picker submitted concurrent duplicate writes")
                current = desired
                checkpoint(f"picker-{mode}-{count}-consistent")
        # An acknowledged local override must not hide a newer external edit.
        api.call("PUT", f"/api/v1/sessions/{sid}", {"favorite": not current[1]})
        current = (current[0], not current[1])
        client.send(b"\x12", "refresh-picker-after-external-edit")
        fixture.poll(lambda: settled(*current), 10, "picker hid a newer server value behind a local override")
        checkpoint("picker-accepts-newer-server-metadata")
        result["status"] = "passed"
    except BaseException:
        result["error"] = traceback.format_exc()
    finally:
        if proxy:
            proxy.writes.set()
            proxy.reads.set()
        if client:
            client.close()
        if proxy:
            proxy.shutdown()
            proxy.server_close()
        if process:
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
