"""Real POSIX PTY interaction companion for http_experience.py.

No keyboard action falls back to an HTTP mutation. HTTP reads only verify
what the real TUI did. OSC 52 is intercepted inside the owned PTY, while the
native clipboard provider is explicitly disabled.
"""

from __future__ import annotations

import base64
import codecs
import errno
import os
from pathlib import Path
import re
import select
import shutil
import signal
import struct
import subprocess
import threading
import time
import traceback
import unicodedata
from typing import Any

# The entry script is also the public-HTTP fixture library. Running the script
# normally names it __main__; reuse that instance instead of importing a second
# copy with a separate provider state.
import __main__ as fixture


ROWS = 36
COLS = 120


class Screen:
    """Bounded VT text projection for assertions; raw ANSI remains authoritative.

    Agena's crossterm renderer uses cursor positioning, erasure, SGR and the
    alternate screen. This projection handles those operations, UTF-8, wide
    characters and split escape sequences. It never executes terminal commands.
    """

    def __init__(self) -> None:
        self.cells = [[" "] * COLS for _ in range(ROWS)]
        self.row = self.col = 0
        self.saved = (0, 0)
        self.pending = ""
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.clipboards: list[bytes] = []

    def feed(self, data: bytes) -> list[bytes]:
        self.pending += self.decoder.decode(data)
        text = self.pending
        index = 0
        replies = []
        while index < len(text):
            char = text[index]
            if char == "\x1b":
                if index + 1 >= len(text):
                    break
                introducer = text[index + 1]
                if introducer == "[":
                    match = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", text[index:])
                    if match is None:
                        break
                    parameters, _, final = match.groups()
                    if final == "n" and parameters == "6":
                        replies.append(f"\x1b[{self.row + 1};{self.col + 1}R".encode())
                    elif final == "u" and parameters == "?":
                        replies.append(b"\x1b[?0u")
                    elif final == "c" and parameters in ("", "0"):
                        replies.append(b"\x1b[?1;2c")
                    else:
                        self.csi(parameters, final)
                    index += len(match.group(0))
                    continue
                if introducer in ("]", "P", "_", "^"):
                    terminator = re.search(r"\x07|\x1b\\", text[index + 2:])
                    if terminator is None:
                        break
                    content = text[index + 2:index + 2 + terminator.start()]
                    if introducer == "]" and content.startswith("11;?"):
                        replies.append(b"\x1b]11;rgb:1010/1010/1010\x1b\\")
                    elif introducer == "]" and content.startswith("10;?"):
                        replies.append(b"\x1b]10;rgb:eeee/eeee/eeee\x1b\\")
                    elif introducer == "]" and content.startswith("52;"):
                        payload = content.split(";", 2)[2]
                        if payload != "?":
                            self.clipboards.append(base64.b64decode(payload, validate=True))
                    index += 2 + terminator.end()
                    continue
                if introducer in ("(", ")"):
                    if index + 2 >= len(text):
                        break
                    index += 3
                    continue
                if introducer == "7":
                    self.saved = self.row, self.col
                elif introducer == "8":
                    self.row, self.col = self.saved
                index += 2
                continue
            if char == "\r":
                self.col = 0
            elif char == "\n":
                self.row += 1
                if self.row >= ROWS:
                    self.cells.pop(0)
                    self.cells.append([" "] * COLS)
                    self.row = ROWS - 1
            elif char == "\b":
                self.col = max(0, self.col - 1)
            elif char == "\t":
                self.col = min(COLS - 1, ((self.col // 8) + 1) * 8)
            elif ord(char) >= 32 and char != "\x7f":
                if unicodedata.combining(char):
                    self.cells[self.row][max(0, self.col - 1)] += char
                else:
                    width = 2 if unicodedata.east_asian_width(char) in ("W", "F") else 1
                    if self.col >= COLS:
                        self.col = 0
                        self.row = min(ROWS - 1, self.row + 1)
                    self.cells[self.row][self.col] = char
                    if width == 2 and self.col + 1 < COLS:
                        self.cells[self.row][self.col + 1] = ""
                    self.col += width
            index += 1
        self.pending = text[index:]
        return replies

    def csi(self, parameters: str, final: str) -> None:
        if parameters.startswith("?"):
            if parameters == "?1049" and final == "h":
                self.cells = [[" "] * COLS for _ in range(ROWS)]
                self.row = self.col = 0
            return
        numbers = [int(value) if value.isdigit() else 0 for value in parameters.split(";")]
        value = numbers[0] if numbers else 0
        distance = value or 1
        if final in ("H", "f"):
            self.row = min(ROWS - 1, max(0, distance - 1))
            self.col = min(COLS - 1, max(0, (numbers[1] if len(numbers) > 1 else 1) - 1))
        elif final == "A":
            self.row = max(0, self.row - distance)
        elif final in ("B", "e"):
            self.row = min(ROWS - 1, self.row + distance)
        elif final in ("C", "a"):
            self.col = min(COLS - 1, self.col + distance)
        elif final == "D":
            self.col = max(0, self.col - distance)
        elif final in ("G", "`"):
            self.col = min(COLS - 1, distance - 1)
        elif final == "d":
            self.row = min(ROWS - 1, distance - 1)
        elif final == "J":
            if value in (2, 3):
                self.cells = [[" "] * COLS for _ in range(ROWS)]
            elif value == 0:
                self.cells[self.row][min(self.col, COLS):] = [" "] * (COLS - min(self.col, COLS))
                for row in range(self.row + 1, ROWS):
                    self.cells[row] = [" "] * COLS
            elif value == 1:
                for row in range(self.row):
                    self.cells[row] = [" "] * COLS
                self.cells[self.row][:min(self.col + 1, COLS)] = [" "] * min(self.col + 1, COLS)
        elif final == "K":
            if value == 2:
                self.cells[self.row] = [" "] * COLS
            elif value == 0:
                self.cells[self.row][min(self.col, COLS):] = [" "] * (COLS - min(self.col, COLS))
            elif value == 1:
                self.cells[self.row][:min(self.col + 1, COLS)] = [" "] * min(self.col + 1, COLS)
        elif final == "s":
            self.saved = self.row, self.col
        elif final == "u":
            self.row, self.col = self.saved

    def text(self) -> str:
        return "\n".join("".join(row).rstrip() for row in self.cells)


class Pty:
    def __init__(self, binary: Path, api: Any, workspace: Path, session_id: int | None,
                 directory: Path, environment: dict[str, str]):
        import fcntl
        import pty
        import termios

        directory.mkdir()
        self.directory = directory
        self.api = api
        self.screen = Screen()
        self.lock = threading.Lock()
        self.write_lock = threading.Lock()
        self.stop = threading.Event()
        self.failure: str | None = None
        self.events = fixture.JsonLines(directory / "events.jsonl")
        self.raw = (directory / "terminal.ansi").open("wb")
        self.master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))
        self.environment = {**environment, "TERM": "xterm-256color", "COLORTERM": "truecolor",
                            "AGENA_RUNTIME_BOOTSTRAP_FORBIDDEN": "1",
                            "AGENA_TUI_NATIVE_CLIPBOARD": "false", "AGENA_TUI_OSC52": "true"}
        self.command = [str(binary), "--server", f"http://127.0.0.1:{api.port}", "tui",
                        "--workspace", str(workspace)]
        if session_id is not None:
            self.command.extend(["--session", str(session_id)])
        started = fixture.now_ns()
        self.process = subprocess.Popen(self.command, cwd=workspace, env=self.environment,
                                        stdin=slave, stdout=slave, stderr=slave, start_new_session=True)
        os.close(slave)
        self.facts: dict[str, Any] = {"pid": self.process.pid, "argv": self.command,
                                     "env": self.environment, "cwd": str(workspace),
                                     "started_ns": started, "spawned_ns": fixture.now_ns(),
                                     "rows": ROWS, "cols": COLS}
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()

    def read(self) -> None:
        try:
            while not self.stop.is_set():
                readable, _, _ = select.select([self.master], [], [], 0.1)
                if not readable:
                    continue
                try:
                    block = os.read(self.master, 65536)
                except OSError as error:
                    if error.errno == errno.EIO:
                        return
                    raise
                if not block:
                    return
                received = fixture.now_ns()
                self.raw.write(block)
                self.raw.flush()
                self.events.add({"kind": "output", "received_ns": received,
                                 "bytes": len(block), "sha256": fixture.digest(block)})
                with self.lock:
                    replies = self.screen.feed(block)
                for reply in replies:
                    self.send(reply, "terminal-query-reply")
        except BaseException:
            if not self.stop.is_set():
                self.failure = traceback.format_exc()

    def send(self, data: bytes, action: str) -> int:
        sent = fixture.now_ns()
        self.events.add({"kind": "input", "action": action, "sent_ns": sent,
                         "bytes_base64": base64.b64encode(data).decode()})
        with self.write_lock:
            offset = 0
            while offset < len(data):
                offset += os.write(self.master, data[offset:])
        return sent

    def text(self) -> str:
        with self.lock:
            return self.screen.text()

    def wait_text(self, marker: str, checkpoint: str) -> dict[str, Any]:
        def visible() -> Any:
            fixture.require(self.process.poll() is None,
                            f"TUI exited {self.process.returncode} before {marker}; screen={self.text()}")
            fixture.require(not self.failure, str(self.failure))
            return {"marker": marker, "observed_ns": fixture.now_ns(), "screen": self.text()} if marker in self.text() else None
        value = fixture.poll(visible, self.api.timeout, f"TUI did not display {marker}; screen={self.text()}")
        (self.directory / f"{checkpoint}.txt").write_text(value["screen"])
        self.events.add({"kind": "checkpoint", "checkpoint": checkpoint, **value})
        return value

    def send_prompt(self, prompt: str) -> int:
        self.send(b"\x1b", "leave-composer-or-selection")
        time.sleep(0.12)
        self.send(b"i", "enter-composer")
        self.send(b"\x1b[200~" + prompt.encode() + b"\x1b[201~", "paste-composer-text")
        return self.send(b"\r", "submit-composer")

    def clipboard(self, expected: bytes) -> dict[str, Any]:
        self.send_prompt("/copy-message")
        def received() -> Any:
            with self.lock:
                values = list(self.screen.clipboards)
            return values[-1] if values else None
        actual = fixture.poll(received, self.api.timeout, "TUI copy-message did not emit an OSC 52 payload")
        (self.directory / "copied.txt").write_bytes(actual)
        fixture.require(actual == expected, "TUI copied bytes differ, including Unicode/whitespace")
        return {"exact_bytes": True, "bytes": len(actual), "sha256": fixture.digest(actual),
                "transport": "captured OSC 52; native clipboard disabled; no outer terminal forwarding"}

    def close(self) -> None:
        self.facts["stop_started_ns"] = fixture.now_ns()
        (self.directory / "screen-at-stop.txt").write_text(self.text())
        if self.process.poll() is None:
            # Two real interrupt keys quit an idle client. Give the first key
            # its own event-loop frame before the confirming key arrives.
            self.send(b"\x1b", "exit-composer-before-quit")
            time.sleep(0.12)
            self.send(b"\x03", "quit-first-ctrl-c")
            time.sleep(0.12)
            self.send(b"\x03", "quit-second-ctrl-c")
            try:
                self.process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGTERM)
                self.process.wait(timeout=3)
                self.facts["forced_termination"] = True
        self.facts.update(exit_code=self.process.returncode, reaped_ns=fixture.now_ns(), reader_error=self.failure)
        self.stop.set()
        self.reader.join(timeout=2)
        os.close(self.master)
        self.raw.close()
        self.events.close()
        fixture.write_json(self.directory / "process.json", self.facts)


def connections(server_pid: int, directory: Path, checkpoint: str) -> dict[str, Any]:
    lsof = shutil.which("lsof") or ("/usr/sbin/lsof" if Path("/usr/sbin/lsof").is_file() else None)
    if lsof is None:
        return {"available": False, "reason": "lsof unavailable"}
    command = [lsof, "-nP", "-a", "-p", str(server_pid), "-iTCP", "-sTCP:ESTABLISHED", "-F", "pn"]
    result = subprocess.run(command, capture_output=True, timeout=5)
    raw = result.stdout.decode(errors="replace")
    (directory / f"connections-{checkpoint}.txt").write_text(raw + result.stderr.decode(errors="replace"))
    return {"available": result.returncode in (0, 1), "argv": command,
            "exit_code": result.returncode, "captured_ns": fixture.now_ns(),
            "connections": [line[1:] for line in raw.splitlines() if line.startswith("n")]}


def verify_sent_document(api: Any, session_id: int, prompt: str) -> None:
    parts = api.parts(session_id)
    fixture.require(any(part["role"] == "user" and prompt in json_text(part["content"])
                        for part in parts), "HTTP transcript does not contain the keyboard-submitted document")


def json_text(value: Any) -> str:
    import json
    return json.dumps(value, ensure_ascii=False)


def run_tui(binary: Path, api: Any, provider: Any, workspace_id: int, workspace: Path,
            directory: Path, environment: dict[str, str], fixture_token: str) -> dict[str, Any]:
    fixture.require(os.name == "posix", "real PTY probe requires POSIX")
    directory.mkdir()
    outcome: dict[str, Any] = {"status": "running", "started_ns": fixture.now_ns(), "flows": []}
    active: Pty | None = None
    record = api.get("/api/v1/health")["server"]
    try:
        for mode in ("send", "permission", "cancel"):
            flow: dict[str, Any] = {"mode": mode, "started_ns": fixture.now_ns()}
            outcome["flows"].append(flow)
            session_id = fixture.create_session(api, workspace_id, f"PTY_{mode.upper()}_FIXTURE")
            flow["session_id"] = session_id
            if mode == "permission":
                fixture.ask_permission(api, session_id)
            flow_dir = directory / mode
            flow_dir.mkdir()
            flow["connections_before"] = connections(record["pid"], flow_dir, "before")
            active = Pty(binary, api, workspace, session_id, flow_dir / "pty", environment)
            flow["ready"] = active.wait_text(f"PTY_{mode.upper()}_FIXTURE", "attached")
            prompt = f"PTY keyboard {mode} fixture"
            if mode == "send":
                text = "TUI_TEXT_BEGIN\n\n" + "".join(
                    f"TUI_ROW_{index:03}  中文 λ  preserved spaces  \n" for index in range(72)) + "\nTUI_TEXT_END\n"
                step = provider.plan("pty_send", "continuation", payloads=[text])
                flow["submit_key_ns"] = active.send_prompt(prompt)
                fixture.require(step.requested.wait(api.timeout), "TUI did not submit to the real provider")
                flow["terminal"] = fixture.wait_idle(api, session_id)
                provider.assert_step(step)
                verify_sent_document(api, session_id, prompt)
                fixture.require(fixture.read_assistant_text(api, session_id) == text, "TUI send HTTP Text differs")
                active.send(b"\x1b", "leave-composer-for-transcript")
                time.sleep(0.12)
                active.send(b"\x1b[13;6u", "expand-all-ctrl-shift-enter")
                active.send(b"gg", "scroll-to-transcript-start")
                flow["first_marker"] = active.wait_text("TUI_TEXT_BEGIN", "scroll-start")
                active.send(b"\x1b[6~", "page-down")
                time.sleep(0.2)
                (active.directory / "page-down.txt").write_text(active.text())
                active.send(b"G", "scroll-to-transcript-end")
                flow["last_marker"] = active.wait_text("TUI_TEXT_END", "scroll-end")
                fixture.require(flow["first_marker"]["screen"] != flow["last_marker"]["screen"],
                                "keyboard scrolling did not change the visible transcript")
                # A completed reply with a body intentionally has no extra
                # lifecycle row. Its actual Answer header uses the completed
                # status icon (●); pending/active use ○/spinner instead.
                active.send(b"gg", "show-completed-answer-header")
                flow["completed_display"] = active.wait_text("▾ ● Answer", "completed")
                active.send(b"G", "restore-transcript-end-before-copy")
                flow["copy"] = active.clipboard(text.encode())
                flow["provider"] = step.snapshot()
            elif mode == "permission":
                tool = provider.plan("pty_permission", "permission")
                followup = provider.plan("pty_permission_continuation", "continuation",
                                         payloads=["TUI_PERMISSION_DONE\n"], expected_tool_bytes=fixture_token)
                flow["submit_key_ns"] = active.send_prompt(prompt)
                pending = fixture.permission_wait(api, session_id)
                flow["pending"] = pending
                flow["permission_display"] = active.wait_text("permission-fixture.txt", "permission-pending")
                # /allow is the TUI's published Allow Once action. Exercise it
                # through the real composer and Enter; never mutate via HTTP.
                flow["reply_action"] = "keyboard /allow followed by Enter"
                flow["reply_key_ns"] = active.send_prompt("/allow")
                fixture.require(followup.requested.wait(api.timeout), "TUI permission key did not resume the actual tool")
                flow["terminal"] = fixture.wait_idle(api, session_id)
                fixture.require(not fixture.pending_permissions(flow["terminal"]), "TUI left the permission gate pending")
                provider.assert_step(tool)
                provider.assert_step(followup)
                verify_sent_document(api, session_id, prompt)
                fixture.require(fixture.read_assistant_text(api, session_id) == "TUI_PERMISSION_DONE\n",
                                "permission TUI continuation did not produce its expected Text")
                active.send(b"\x1b", "leave-permission-control")
                time.sleep(0.12)
                active.send(b"G", "show-permission-continuation")
                flow["completion_display"] = active.wait_text("TUI_PERMISSION_DONE", "permission-completed")
                flow["provider"] = followup.snapshot()
            else:
                held = provider.plan("pty_cancel", "held", payloads=["TUI_CANCEL_PARTIAL\n"])
                flow["submit_key_ns"] = active.send_prompt(prompt)
                flow["partial_display"] = active.wait_text("TUI_CANCEL_PARTIAL", "cancel-partial")
                running = api.state(session_id)
                fixture.require(fixture.state_kind(running) == "running", "cancel fixture was not actually running")
                flow["running"] = running
                flow["cancel_key_ns"] = active.send(b"\x03", "cancel-active-run-ctrl-c")
                flow["terminal"] = fixture.wait_idle(api, session_id)
                fixture.require(held.disconnected.wait(api.timeout), "TUI cancellation did not disconnect the provider")
                provider.assert_step(held)
                verify_sent_document(api, session_id, prompt)
                active.send(b"\x1b", "leave-composer-after-cancel")
                time.sleep(0.12)
                active.send(b"G", "show-cancelled-response")
                flow["cancel_display"] = active.wait_text("Response cancelled", "cancel-completed")
                flow["provider"] = held.snapshot()
            flow["process"] = active.facts
            active.close()
            active = None
            baseline = flow["connections_before"]
            def released() -> Any:
                current = connections(record["pid"], flow_dir, "after")
                if not baseline.get("available") or not current.get("available"):
                    return current
                return current if len(current["connections"]) <= len(baseline["connections"]) else None
            flow["connections_after"] = fixture.poll(released, api.timeout, "TUI connections remained after process exit")
            flow.update(status="passed", completed_ns=fixture.now_ns())
            fixture.write_json(flow_dir / "flow.json", flow)
        outcome["status"] = "passed"
    except BaseException:
        outcome.update(status="failed", error=traceback.format_exc())
        raise
    finally:
        if active is not None:
            active.close()
        outcome["completed_ns"] = fixture.now_ns()
        fixture.write_json(directory / "result.json", outcome)
    return outcome
