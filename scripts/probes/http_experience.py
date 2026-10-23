#!/usr/bin/env python3
"""Measure a real Agena process through its public HTTP API, using no providers.

The only provider is an in-process loopback Responses fixture. This is a
measurement harness, not a production transport or a compatibility adapter.
Run this exact file against each separately built binary; see the adjacent
README for the workload, timing boundaries, and evidence format.
"""

from __future__ import annotations

import argparse
import dataclasses
import datetime as dt
import hashlib
import http.client
import http.server
import json
import math
import os
from pathlib import Path
import queue
import signal
import socket
import subprocess
import sys
import threading
import time
import traceback
from typing import Any, Callable
from urllib.parse import urlsplit


APPENDS = 100
APPEND_BYTES = 1024
SPACING_SECONDS = 0.002
DISCOVERY_SECONDS = 0.005
PAYLOADS = [f"{index:03} {'x' * 1019}\n" for index in range(APPENDS)]


def now_ns() -> int:
    return time.perf_counter_ns()


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def file_digest(path: Path) -> str:
    hasher = hashlib.sha256()
    with path.open("rb") as source:
        while block := source.read(1024 * 1024):
            hasher.update(block)
    return hasher.hexdigest()


def write_json(path: Path, value: Any) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")
    temporary.replace(path)


def require(condition: Any, message: str) -> None:
    if not condition:
        raise RuntimeError(message)


def distribution(values: list[float]) -> dict[str, Any]:
    ordered = sorted(values)
    if not ordered:
        return {"samples": 0, "p50_ms": None, "p95_ms": None, "max_ms": None}
    return {
        "samples": len(ordered),
        "p50_ms": ordered[max(0, math.ceil(0.50 * len(ordered)) - 1)],
        "p95_ms": ordered[max(0, math.ceil(0.95 * len(ordered)) - 1)],
        "max_ms": ordered[-1],
    }


class JsonLines:
    def __init__(self, path: Path):
        self.file = path.open("w", encoding="utf-8")
        self.lock = threading.Lock()

    def add(self, value: Any) -> None:
        with self.lock:
            self.file.write(json.dumps(value, ensure_ascii=False, separators=(",", ":")) + "\n")
            self.file.flush()

    def close(self) -> None:
        with self.lock:
            self.file.close()


class Api:
    """One persistent command connection per thread; SSE owns separate sockets."""

    def __init__(self, base_url: str, timeout: float, trace: JsonLines):
        endpoint = urlsplit(base_url)
        require(endpoint.scheme == "http" and endpoint.hostname == "127.0.0.1", "loopback HTTP required")
        self.host = endpoint.hostname
        self.port = endpoint.port
        self.timeout = timeout
        self.trace = trace
        self.local = threading.local()

    def connection(self) -> http.client.HTTPConnection:
        return http.client.HTTPConnection(self.host, self.port, timeout=self.timeout)

    def call(self, method: str, path: str, body: Any = None) -> tuple[Any, dict[str, Any]]:
        connection = getattr(self.local, "connection", None)
        if connection is None:
            connection = self.connection()
            self.local.connection = connection
        encoded = None if body is None else json.dumps(body, separators=(",", ":")).encode()
        started = now_ns()
        record: dict[str, Any] = {"method": method, "path": path, "started_ns": started, "request": body}
        try:
            connection.request(method, path, encoded, {"Content-Type": "application/json"})
            response = connection.getresponse()
            record["headers_received_ns"] = now_ns()
            raw = response.read()
            record.update(status=response.status, completed_ns=now_ns(), response_bytes=len(raw))
            decoded = json.loads(raw)
            record["response"] = decoded
            require(200 <= response.status < 300, f"{method} {path}: HTTP {response.status}: {decoded}")
            record["confirmation_ms"] = (record["completed_ns"] - started) / 1e6
            return decoded, record
        except BaseException as error:
            record.update(error=str(error), completed_ns=now_ns())
            connection.close()
            self.local.connection = None
            raise
        finally:
            self.trace.add({"kind": "http", **record})

    def get(self, path: str) -> Any:
        return self.call("GET", path)[0]

    def state(self, session_id: int) -> Any:
        return self.get(f"/api/v1/sessions/{session_id}/state")

    def parts(self, session_id: int) -> list[dict[str, Any]]:
        response = self.get(f"/api/v1/sessions/{session_id}/parts?limit=256")
        require(not response["page"]["has_more"], "fixture unexpectedly exceeded a single transcript page")
        return response["parts"]


def state_kind(execution: dict[str, Any]) -> str:
    return execution["session"]["state"]["kind"]


def pending_permissions(execution: dict[str, Any]) -> list[dict[str, Any]]:
    return [item for item in execution["session"]["state"].get("data", {}).get("requests", [])
            if item.get("kind") == "permission"]


def poll(predicate: Callable[[], Any], timeout: float, description: str) -> Any:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if value := predicate():
            return value
        time.sleep(0.01)
    raise TimeoutError(description)


def wait_idle(api: Api, session_id: int) -> Any:
    def ready() -> Any:
        state = api.state(session_id)
        require(state_kind(state) != "failed", f"fixture execution failed: {state}")
        return state if state_kind(state) == "ready" else None
    return poll(ready, api.timeout, f"session {session_id} did not become ready")


def validate_receipt(response: dict[str, Any], session_id: int) -> dict[str, Any]:
    require(response["session"]["id"] == session_id, "send confirmation has the wrong session")
    receipt = response.get("receipt")
    require(receipt and all(receipt.get(key) for key in ("execution_id", "turn_id", "reply_id")),
            "send confirmation must contain the accepted execution/turn/reply receipt")
    return receipt


@dataclasses.dataclass
class ProviderStep:
    name: str
    mode: str
    payloads: list[str] = dataclasses.field(default_factory=list)
    expected_tool_bytes: str | None = None
    fixture_path: str = "permission-fixture.txt"
    requested: threading.Event = dataclasses.field(default_factory=threading.Event)
    steady_gate: threading.Event = dataclasses.field(default_factory=threading.Event)
    finished: threading.Event = dataclasses.field(default_factory=threading.Event)
    disconnected: threading.Event = dataclasses.field(default_factory=threading.Event)
    samples: list[dict[str, Any]] = dataclasses.field(default_factory=list)
    facts: dict[str, Any] = dataclasses.field(default_factory=dict)
    error: str | None = None

    def snapshot(self) -> dict[str, Any]:
        return {"name": self.name, "mode": self.mode, "samples": self.samples,
                "facts": self.facts, "error": self.error}


class Provider:
    def __init__(self, timeout: float, trace: JsonLines):
        self.timeout = timeout
        self.trace = trace
        self.steps: list[ProviderStep] = []
        self.pending: queue.Queue[ProviderStep] = queue.Queue()
        self.errors: list[str] = []
        fixture = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_: Any) -> None:
                pass

            def do_GET(self) -> None:
                raw = json.dumps({"object": "list", "data": [{"id": "fake-model", "object": "model"}]}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

            def do_POST(self) -> None:
                if self.path.rstrip("/") != "/v1/responses":
                    fixture.errors.append(f"unexpected provider POST {self.path}")
                    self.send_error(404)
                    return
                try:
                    step = fixture.pending.get(timeout=fixture.timeout)
                except queue.Empty:
                    fixture.errors.append("unplanned provider request")
                    self.send_error(500)
                    return
                try:
                    body = self.rfile.read(int(self.headers["Content-Length"]))
                    request = json.loads(body)
                    step.facts.update(request_received_ns=now_ns(), request_sha256=digest(body),
                                      request_bytes=len(body))
                    fixture.trace.add({"kind": "provider_request", "step": step.name,
                                       "received_ns": step.facts["request_received_ns"], "request": request})
                    if step.expected_tool_bytes is not None:
                        require(step.expected_tool_bytes in json.dumps(request, ensure_ascii=False),
                                "permission continuation did not contain the actual fs.read fixture bytes")
                        step.facts["actual_fs_read_bytes_verified"] = True
                    step.requested.set()
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream")
                    self.send_header("Cache-Control", "no-cache")
                    self.send_header("Connection", "close")
                    self.end_headers()
                    self.close_connection = True
                    self.connection.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
                    if step.mode == "permission":
                        call = {"type": "function_call", "id": f"fc_{step.name}",
                                "call_id": f"call_{step.name}", "name": "tools_call"}
                        arguments = json.dumps({"tool": "fs.read", "input": {"file_path": step.fixture_path}})
                        self.event({"type": "response.output_item.added", "output_index": 0,
                                    "item": {**call, "arguments": ""}})
                        self.event({"type": "response.output_item.done", "output_index": 0,
                                    "item": {**call, "arguments": arguments}})
                        self.complete(step, tool=True)
                    else:
                        for index, payload in enumerate(step.payloads):
                            if index:
                                if index == 1 and step.mode == "text":
                                    require(step.steady_gate.wait(fixture.timeout),
                                            "observers did not consume the bootstrap marker")
                                    step.facts["steady_barrier_released_ns"] = now_ns()
                                time.sleep(SPACING_SECONDS)
                            sample = {"index": index, "bytes": len(payload.encode()),
                                      "sha256": digest(payload.encode()), "send_started_ns": now_ns()}
                            step.samples.append(sample)
                            self.event({"type": "response.output_text.delta", "delta": payload})
                            sample["send_completed_ns"] = now_ns()
                        if step.mode == "held":
                            self.connection.settimeout(fixture.timeout)
                            while self.connection.recv(1024):
                                pass
                            step.facts["disconnect_observed_ns"] = now_ns()
                            step.disconnected.set()
                        else:
                            self.complete(step)
                except BaseException as error:
                    step.error = f"{type(error).__name__}: {error}"
                    fixture.errors.append(f"{step.name}: {step.error}")
                    fixture.trace.add({"kind": "provider_error", "step": step.name, "error": step.error})
                finally:
                    step.facts["handler_finished_ns"] = now_ns()
                    fixture.trace.add({"kind": "provider_step", **step.snapshot()})
                    step.finished.set()

            def event(self, value: dict[str, Any]) -> None:
                self.wfile.write(b"data: " + json.dumps(value, separators=(",", ":")).encode() + b"\n\n")
                self.wfile.flush()

            def complete(self, step: ProviderStep, tool: bool = False) -> None:
                response = {"id": f"resp_{step.name}", "status": "completed",
                            "usage": {"input_tokens": 5, "output_tokens": 3, "total_tokens": 8}}
                if tool:
                    response["stop_reason"] = "tool_calls"
                self.event({"type": "response.completed", "response": response})
                step.facts["response_completed_ns"] = now_ns()

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.base_url = f"http://127.0.0.1:{self.server.server_port}/v1"

    def plan(self, name: str, mode: str, **options: Any) -> ProviderStep:
        step = ProviderStep(name=name, mode=mode, **options)
        self.steps.append(step)
        self.pending.put(step)
        return step

    def assert_step(self, step: ProviderStep) -> None:
        require(step.finished.wait(self.timeout), f"provider step {step.name} did not finish")
        require(not step.error, f"provider step {step.name}: {step.error}")

    def close(self) -> None:
        for step in self.steps:
            step.steady_gate.set()
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)


class ContentObserver:
    def __init__(self, api: Api, session_id: int, resource_id: str, step: ProviderStep,
                 index: int, directory: Path):
        self.api, self.step, self.index = api, step, index
        self.session_id, self.resource_id = session_id, resource_id
        self.directory = directory
        self.first = threading.Event()
        self.done = threading.Event()
        self.stop = threading.Event()
        self.connection = api.connection()
        self.transport_socket: socket.socket | None = None
        self.error: str | None = None
        self.data = bytearray()
        self.arrivals: list[dict[str, Any]] = []
        self.pages: list[dict[str, Any]] = []
        self.expected = "".join(step.payloads).encode()
        self.opened_ns: int | None = None
        self.thread = threading.Thread(target=self.run, daemon=True)
        self.thread.start()

    def run(self) -> None:
        output = self.directory / f"observer-{self.index:02}.jsonl"
        try:
            path = f"/api/v1/sessions/{self.session_id}/content/{self.resource_id}/stream?max_bytes=65536"
            self.connection.request("GET", path, headers={"Accept": "text/event-stream"})
            # HTTPConnection relinquishes its socket when the response says
            # Connection: close; retain the owned socket to interrupt a reader.
            self.transport_socket = self.connection.sock
            response = self.connection.getresponse()
            require(response.status == 200, f"content SSE returned HTTP {response.status}")
            self.opened_ns = now_ns()
            epoch: str | None = None
            cursor = 0
            with output.open("w", encoding="utf-8") as trace:
                event_name = "message"
                event_data: list[str] = []
                while not self.stop.is_set():
                    line = response.readline(4 * 1024 * 1024)
                    if not line:
                        break
                    line = line.rstrip(b"\r\n")
                    if line.startswith(b"event:"):
                        event_name = line[6:].strip().decode()
                    elif line.startswith(b"data:"):
                        event_data.append(line[5:].lstrip(b" ").decode())
                    elif not line and event_data:
                        received = now_ns()
                        value = json.loads("\n".join(event_data))
                        trace.write(json.dumps({"received_ns": received, "event": event_name,
                                                "data": value}, ensure_ascii=False) + "\n")
                        trace.flush()
                        require(event_name == "content", f"unexpected content event {event_name}: {value}")
                        resource = value["resource"]
                        require(not value["gap"] and resource["dropped_bytes"] == 0,
                                "content retention gap or dropped bytes")
                        current_epoch = resource["cursor"]["epoch"]
                        require(epoch is None or current_epoch == epoch, "unexpected content epoch change")
                        epoch = current_epoch
                        self.pages.append({"received_ns": received, "chunks": len(value["chunks"]),
                                           "resource_cursor": resource["cursor"],
                                           "next_cursor": value["next_cursor"], "state": resource["state"]})
                        for chunk in value["chunks"]:
                            require(chunk["cursor"]["epoch"] == epoch, "chunk belongs to the wrong epoch")
                            sequence = chunk["cursor"]["sequence"]
                            require(sequence > cursor, "duplicate or reordered content chunk")
                            cursor = sequence
                            require(chunk["payload"]["type"] == "text", "fixture received non-text content")
                            text = chunk["payload"]["text"].encode()
                            before = len(self.data)
                            self.data.extend(text)
                            require(self.expected.startswith(self.data),
                                    f"observer {self.index}: content differs at byte {before}")
                            self.arrivals.append({"received_ns": received, "start_byte": before,
                                                  "end_byte": len(self.data), "cursor": chunk["cursor"]})
                            if text:
                                self.first.set()
                            if len(self.data) == len(self.expected):
                                self.done.set()
                        event_name, event_data = "message", []
            require(self.stop.is_set() or self.done.is_set(), "content SSE ended before all marker bytes arrived")
        except BaseException as error:
            if not self.stop.is_set():
                self.error = f"{type(error).__name__}: {error}"
        finally:
            (self.directory / f"observer-{self.index:02}.txt").write_bytes(self.data)
            self.connection.close()
            self.done.set()

    def snapshot(self) -> dict[str, Any]:
        offset = 0
        samples = []
        for source in self.step.samples:
            end = offset + source["bytes"]
            first = next((row["received_ns"] for row in self.arrivals if row["end_byte"] > offset), None)
            complete = next((row["received_ns"] for row in self.arrivals if row["end_byte"] >= end), None)
            samples.append({**source, "start_byte": offset, "end_byte": end,
                            "first_byte_received_ns": first, "complete_received_ns": complete,
                            "first_byte_delivery_ms": None if first is None else (first - source["send_started_ns"]) / 1e6,
                            "complete_delivery_ms": None if complete is None else (complete - source["send_started_ns"]) / 1e6})
            offset = end
        return {"observer": self.index, "resource_id": self.resource_id, "opened_ns": self.opened_ns,
                "bytes": len(self.data), "expected_bytes": len(self.expected),
                "sha256": digest(self.data), "expected_sha256": digest(self.expected),
                "exact_bytes": bytes(self.data) == self.expected, "error": self.error,
                "provider_delta_samples": samples, "content_arrivals": self.arrivals, "pages": self.pages,
                "raw_sse": str((self.directory / f"observer-{self.index:02}.jsonl").name),
                "raw_text": str((self.directory / f"observer-{self.index:02}.txt").name)}

    def close(self) -> None:
        self.stop.set()
        connection_socket = self.transport_socket
        if connection_socket is not None:
            try:
                connection_socket.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
        self.connection.close()
        self.thread.join(timeout=2)
        require(not self.thread.is_alive(), "content observer did not stop")


class ObserverGroup:
    def __init__(self, api: Api, session_id: int, step: ProviderStep, count: int, directory: Path):
        self.api, self.session_id, self.step, self.count = api, session_id, step, count
        self.directory = directory
        self.observers: list[ContentObserver] = []
        self.error: str | None = None
        self.stop = threading.Event()
        self.ready = threading.Event()
        self.thread = threading.Thread(target=self.discover, daemon=True)
        self.thread.start()

    def discover(self) -> None:
        try:
            deadline = time.monotonic() + self.api.timeout
            while not self.stop.is_set():
                resources = [reference for part in self.api.parts(self.session_id)
                             if part["role"] == "assistant" and part["kind"] == "text"
                             for reference in part["content"].get("resources", [])
                             if reference["kind"] == "text"]
                if resources:
                    require(len(resources) == 1, "fixture expected one assistant Text resource")
                    for index in range(self.count):
                        self.observers.append(ContentObserver(self.api, self.session_id,
                                                             resources[0]["resource_id"], self.step,
                                                             index, self.directory))
                    for observer in self.observers:
                        require(observer.first.wait(self.api.timeout), "content SSE did not deliver its first marker")
                        require(not observer.error, str(observer.error))
                    self.step.steady_gate.set()
                    self.ready.set()
                    return
                require(time.monotonic() < deadline, "assistant Text resource was not published")
                time.sleep(DISCOVERY_SECONDS)
        except BaseException as error:
            self.error = f"{type(error).__name__}: {error}"
            self.ready.set()

    def wait(self) -> list[dict[str, Any]]:
        require(self.ready.wait(self.api.timeout), "observers did not attach")
        require(not self.error, str(self.error))
        for observer in self.observers:
            require(observer.done.wait(self.api.timeout), "content SSE did not deliver all marker bytes")
            require(not observer.error, str(observer.error))
            require(bytes(observer.data) == observer.expected, "content byte integrity check failed")
        return [observer.snapshot() for observer in self.observers]

    def close(self) -> None:
        self.stop.set()
        self.step.steady_gate.set()
        for observer in self.observers:
            observer.first.set()
        self.thread.join(timeout=self.api.timeout + 1)
        for observer in self.observers:
            observer.close()


def create_session(api: Api, workspace_id: int, title: str) -> int:
    return api.call("POST", "/api/v1/sessions", {"workspace_id": workspace_id,
                    "title": title, "parent_id": None})[0]["id"]


def submit(api: Api, session_id: int, prompt: str) -> tuple[Any, Any]:
    response, timing = api.call("POST", f"/api/v1/sessions/{session_id}/messages",
                                {"document": [{"type": "text", "text": prompt}]})
    validate_receipt(response, session_id)
    return response, timing


def ask_permission(api: Api, session_id: int) -> None:
    api.call("PUT", f"/api/v1/sessions/{session_id}/permission",
             {"permission": {"path": {"workspace": {"read": "ask", "write": "ask"},
                                        "rules": {"permission-fixture.txt": {"read": "ask", "write": "ask"}}}}})


def permission_wait(api: Api, session_id: int) -> Any:
    def pending() -> Any:
        execution = api.state(session_id)
        require(state_kind(execution) != "failed", f"permission fixture failed: {execution}")
        requests = pending_permissions(execution)
        require(len(requests) <= 1, "permission fixture raised multiple interactions")
        return requests[0] if requests else None
    return poll(pending, api.timeout, "real fs.read did not request permission")


def read_assistant_text(api: Api, session_id: int) -> str:
    texts = []
    for part in api.parts(session_id):
        if part["role"] != "assistant" or part["kind"] != "text":
            continue
        for reference in part["content"].get("resources", []):
            if reference["kind"] != "text":
                continue
            page = api.get(f"/api/v1/sessions/{session_id}/content/{reference['resource_id']}?max_bytes=1048576")
            require(not page["gap"] and not page["has_more"], "fixture text read was incomplete")
            texts.extend(chunk["payload"]["text"] for chunk in page["chunks"]
                         if chunk["payload"]["type"] == "text")
        texts.append(part["content"].get("text", ""))
    return "".join(texts)


def run_trial(api: Api, provider: Provider, workspace_id: int, observers: int,
              trial_index: int, directory: Path, fixture_token: str) -> dict[str, Any]:
    directory.mkdir()
    trial: dict[str, Any] = {"trial": trial_index, "observers": observers,
                             "started_ns": now_ns(), "status": "running"}
    group: ObserverGroup | None = None
    try:
        name = f"o{observers}_t{trial_index:02}"
        session_id = create_session(api, workspace_id, f"HTTP Text {name}")
        trial["text_session_id"] = session_id
        step = provider.plan(f"{name}_text", "text", payloads=PAYLOADS)
        group = ObserverGroup(api, session_id, step, observers, directory)
        response, timing = submit(api, session_id, f"controlled Text markers {name}")
        trial["send"] = {"timing": timing, "receipt": response["receipt"]}
        trial["content"] = group.wait()
        for observer in trial["content"]:
            first_arrival = observer["provider_delta_samples"][0]["first_byte_received_ns"]
            observer["submit_to_first_content_ms"] = (first_arrival - timing["started_ns"]) / 1e6
        trial["terminal"] = wait_idle(api, session_id)
        provider.assert_step(step)
        trial["provider_text"] = step.snapshot()
        group.close()
        group = None

        cancel_id = create_session(api, workspace_id, f"HTTP cancel {name}")
        trial["cancel_session_id"] = cancel_id
        held = provider.plan(f"{name}_cancel", "held", payloads=[f"CANCEL_{name}\n"])
        cancel_dir = directory / "cancel"
        cancel_dir.mkdir()
        group = ObserverGroup(api, cancel_id, held, observers, cancel_dir)
        accepted, cancel_send = submit(api, cancel_id, f"hold until cancellation {name}")
        group.wait()
        result, cancellation = api.call("POST", f"/api/v1/sessions/{cancel_id}/cancel",
                                        {"execution_id": accepted["receipt"]["execution_id"]})
        require(result["result"] == "cancellation_requested", f"unexpected cancellation result: {result}")
        terminal = wait_idle(api, cancel_id)
        terminal_observed = now_ns()
        require(held.disconnected.wait(api.timeout), "cancel did not disconnect the held provider")
        provider.assert_step(held)
        trial["cancel"] = {"send_timing": cancel_send, "timing": cancellation, "outcome": result,
                           "terminal": terminal, "terminal_observed_ns": terminal_observed,
                           "terminal_ms": (terminal_observed - cancellation["started_ns"]) / 1e6,
                           "provider": held.snapshot(), "content": group.wait()}
        group.close()
        group = None

        permission_id = create_session(api, workspace_id, f"HTTP real fs.read {name}")
        trial["permission_session_id"] = permission_id
        ask_permission(api, permission_id)
        tool = provider.plan(f"{name}_permission", "permission")
        continuation = provider.plan(f"{name}_continuation", "continuation",
                                     payloads=[f"PERMISSION_DONE_{name}\n"], expected_tool_bytes=fixture_token)
        _, permission_send = submit(api, permission_id, f"read permission-fixture.txt {name}")
        pending = permission_wait(api, permission_id)
        pending_observed = now_ns()
        reply, reply_timing = api.call("POST", f"/api/v1/sessions/{permission_id}/permission-replies",
                                       {"reply": {"request_id": pending["request_id"], "kind": "allow_once",
                                                   "reason": "controlled HTTP experience fixture"}})
        terminal = wait_idle(api, permission_id)
        require(not pending_permissions(terminal), "permission interaction remained pending after reply")
        provider.assert_step(tool)
        provider.assert_step(continuation)
        actual_text = read_assistant_text(api, permission_id)
        require(actual_text == "".join(continuation.payloads), "permission continuation Text differs")
        trial["permission"] = {"send_timing": permission_send, "pending": pending,
                               "pending_observed_ns": pending_observed, "reply_timing": reply_timing,
                               "reply_response": reply, "terminal": terminal, "text": actual_text,
                               "tool_provider": tool.snapshot(), "continuation_provider": continuation.snapshot()}
        trial["status"] = "passed"
    except BaseException:
        trial.update(status="failed", error=traceback.format_exc())
        raise
    finally:
        if group is not None:
            trial["partial_content"] = [observer.snapshot() for observer in group.observers]
            group.close()
        trial["completed_ns"] = now_ns()
        write_json(directory / "trial.json", trial)
    return trial


def summarize(trials: list[dict[str, Any]]) -> dict[str, Any]:
    summaries = {}
    for count in sorted({trial["observers"] for trial in trials}):
        selected = [trial for trial in trials if trial["observers"] == count]
        content = [observer for trial in selected for observer in trial["content"]]
        first = [observer["provider_delta_samples"][0]["first_byte_delivery_ms"] for observer in content]
        incremental = [sample["complete_delivery_ms"] for observer in content
                       for sample in observer["provider_delta_samples"][1:]]
        summaries[str(count)] = {
            "trials": len(selected),
            "send_confirmation": distribution([trial["send"]["timing"]["confirmation_ms"] for trial in selected]),
            "provider_first_delta_to_content_sse_first_byte": distribution(first),
            "submit_to_first_content_sse_first_byte": distribution([observer["submit_to_first_content_ms"] for observer in content]),
            "per_trial_slowest_first_byte": distribution([
                max(observer["provider_delta_samples"][0]["first_byte_delivery_ms"] for observer in trial["content"])
                for trial in selected]),
            "provider_incremental_delta_to_content_sse_complete": distribution(incremental),
            "cancel_confirmation": distribution([trial["cancel"]["timing"]["confirmation_ms"] for trial in selected]),
            "cancel_to_terminal": distribution([trial["cancel"]["terminal_ms"] for trial in selected]),
            "permission_reply_confirmation": distribution([trial["permission"]["reply_timing"]["confirmation_ms"] for trial in selected]),
            "exact_content_observers": sum(observer["exact_bytes"] for observer in content),
            "expected_content_observers": len(content),
        }
    return summaries


def clean_environment(directory: Path) -> dict[str, str]:
    data_dir = directory / "data"
    require(data_dir.is_dir(), "owned Agena data directory must be created by this run")
    env = {key: os.environ[key] for key in ("PATH", "HOME", "USERPROFILE", "TMPDIR", "TMP", "TEMP", "SYSTEMROOT", "RUST_MIN_STACK")
           if key in os.environ}
    env.update({"LANG": "en_US.UTF-8", "LC_ALL": "en_US.UTF-8", "NO_PROXY": "127.0.0.1,localhost",
                "AGENA_DATA_DIR": str(data_dir),
                "AGENA_RUNTIME_OWNERSHIP_AUDIT_PATH": str(data_dir / "run" / "runtime-ownership.jsonl")})
    return env


def prepare_configuration(directory: Path, provider_url: str) -> Path:
    workspace = directory / "workspace"
    (workspace / ".agena").mkdir(parents=True)
    # One owned data root isolates config, database, credentials, artifacts and
    # discovery records without redirecting the user's operating-system home.
    global_config = directory / "data" / "agena.json"
    global_config.parent.mkdir(parents=True, mode=0o700)
    (directory / "process-cwd").mkdir()
    write_json(global_config, {"ui": {"locale": "en-US"},
                               "runtime": {"providers": {"client_versions": {"auto_update": False}}}})
    write_json(workspace / ".agena" / "agena.json", {
        "ui": {"locale": "en-US"},
        "providers": {"default_selection": {"provider": "fake", "adapter": "openai_responses", "model": "fake-model"},
                      "fake": {"auth": {"mode": "api", "subtype": "custom", "base_url": provider_url,
                                        "api_key": {"kind": "inline", "value": "local-experience-fixture-only"}},
                               "adapters": {"openai_responses": {"enabled": True,
                                             "models": {"fake-model": {"agena_tools": {"mode": "provider_protocol"}}}}}}}})
    return workspace


def wait_server(process: subprocess.Popen[bytes], api: Api, record_path: Path) -> Any:
    def available() -> Any:
        require(process.poll() is None, f"server exited before ready with code {process.returncode}")
        if not record_path.is_file():
            return None
        try:
            record = json.loads(record_path.read_text())
            health = api.get("/api/v1/health")
        except (OSError, ValueError, http.client.HTTPException):
            return None
        require(record["pid"] == process.pid and health["server"]["pid"] == process.pid,
                "public endpoint does not belong to the launched process")
        require(record["schema"] == 1 and record["server_id"] == health["server"]["id"]
                and record["started_at"] == health["server"]["started_at"]
                and record["protocol_version"] == health["server"]["protocol_version"],
                "server record and HTTP identity disagree")
        return {"record": record, "health": health, "ready_ns": now_ns()}
    return poll(available, api.timeout, "server did not publish a matching healthy endpoint")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path, help="new owned directory; existing output is refused")
    parser.add_argument("--label", required=True)
    parser.add_argument("--source-revision", required=True)
    parser.add_argument("--trials", type=int, default=20)
    parser.add_argument("--observers", default="1,8")
    parser.add_argument("--timeout", type=float, default=30)
    parser.add_argument("--tui", action="store_true", help="also drive real send/permission/cancel through a PTY")
    parser.add_argument("--tui-only", action="store_true", help="run the real PTY flows without the HTTP latency batch")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    observer_counts = [int(value) for value in args.observers.split(",")]
    require(args.trials > 0 and observer_counts and all(value > 0 for value in observer_counts), "invalid workload")
    parent_home = os.environ.get("HOME")
    write_json(output / "ownership.json", {"owner": "agena-http-experience", "pid": os.getpid(),
                                           "created_at": utc_now(), "directory": str(output)})
    (output / "payload.txt").write_bytes("".join(PAYLOADS).encode())
    trace = JsonLines(output / "http-provider.jsonl")
    provider = Provider(args.timeout, trace)
    process: subprocess.Popen[bytes] | None = None
    process_log = (output / "server.log").open("wb")
    result: dict[str, Any] = {
        "schema": 1, "label": args.label, "source_revision": args.source_revision,
        "status": "running", "started_at": utc_now(), "started_ns": now_ns(),
        "harness_pid": os.getpid(), "python": sys.version, "platform": sys.platform,
        "binary": {"path": str(binary), "sha256": file_digest(binary), "bytes": binary.stat().st_size},
        "harness_sources": {path.name: file_digest(path) for path in
                            [Path(__file__).resolve(), Path(__file__).with_name("tui_experience.py").resolve()]
                            if path.is_file()},
        "workload": {"trials_per_observer_count": args.trials, "observers": observer_counts,
                     "provider_deltas": APPENDS, "delta_bytes": APPEND_BYTES,
                     "source_bytes": APPENDS * APPEND_BYTES, "delta_spacing_ms": SPACING_SECONDS * 1000,
                     "resource_discovery_interval_ms": DISCOVERY_SECONDS * 1000,
                     "bootstrap_barrier": "after delta 0, wait until every observer consumes it; then deltas 1..99 at 2 ms requested spacing",
                     "payload_sha256": digest("".join(PAYLOADS).encode()),
                     "timing_clock": "same-process perf_counter_ns for provider and HTTP/SSE consumers",
                     "p95_method": "nearest rank, ceil(0.95*n)",
                     "content_timing_boundary": "complete public SSE event received, before JSON decode; marker bytes verified after decode"},
        "trials": [], "provider_base_url": provider.base_url,
    }
    exit_code = 1
    try:
        workspace = prepare_configuration(output, provider.base_url)
        fixture_token = f"AGENA_REAL_FS_READ_{digest(str(output).encode())[:24]}"
        (workspace / "permission-fixture.txt").write_text(fixture_token + "\n")
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        env = clean_environment(output)
        command = [str(binary), "--data-dir", str(output / "data"),
                   "server", "--host", "127.0.0.1", "--port", str(port), "--workspace", str(workspace)]
        result["process"] = {"argv": command, "cwd": str(output / "process-cwd"), "env": env,
                             "spawn_started_ns": now_ns(), "spawn_started_at": utc_now()}
        process = subprocess.Popen(command, cwd=output / "process-cwd", env=env, stdin=subprocess.DEVNULL,
                                   stdout=process_log, stderr=subprocess.STDOUT, start_new_session=True)
        result["process"].update(pid=process.pid, spawn_completed_ns=now_ns())
        api = Api(f"http://127.0.0.1:{port}", args.timeout, trace)
        result["identity"] = wait_server(process, api, output / "data" / "run" / "server.json")
        workspace_response = api.call("POST", "/api/v1/workspaces/resolve",
                                      {"path": str(workspace), "create_if_missing": True})[0]
        workspace_id = workspace_response["id"]
        result["workspace"] = workspace_response
        if not args.tui_only:
            for count in observer_counts:
                for trial_index in range(args.trials):
                    trial_dir = output / f"http-o{count}-t{trial_index:02}"
                    trial = run_trial(api, provider, workspace_id, count, trial_index, trial_dir, fixture_token)
                    result["trials"].append({**trial, "artifact_directory": trial_dir.name})
                    write_json(output / "result.json", result)
        if args.tui or args.tui_only:
            from tui_experience import run_tui
            result["tui"] = run_tui(binary, api, provider, workspace_id, workspace, output / "tui", env, fixture_token)
        require(not provider.errors, f"fake provider errors: {provider.errors}")
        require(provider.pending.empty(), "planned provider steps were not consumed")
        result["summary"] = summarize(result["trials"])
        result["status"] = "passed"
        exit_code = 0
    except BaseException:
        result.update(status="failed", error=traceback.format_exc())
    finally:
        result["provider_steps"] = [step.snapshot() for step in provider.steps]
        result["provider_errors"] = provider.errors
        if process is not None:
            result["process"]["stop_started_ns"] = now_ns()
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait(timeout=5)
                    result["process"]["forced_kill"] = True
            result["process"].update(exit_code=process.returncode, reaped_ns=now_ns(), reaped_at=utc_now())
        provider.close()
        process_log.close()
        trace.close()
        result["parent_home_unchanged"] = os.environ.get("HOME") == parent_home
        result.update(completed_ns=now_ns(), completed_at=utc_now())
        write_json(output / "result.json", result)
        write_json(output / "artifact-manifest.json", {
            "schema": 1, "created_at": utc_now(), "status": result["status"],
            "files": [{"path": str(path.relative_to(output)), "bytes": path.stat().st_size,
                       "sha256": file_digest(path)} for path in sorted(output.rglob("*"))
                      if path.is_file() and path.name != "artifact-manifest.json"]})
    print(json.dumps({"status": result["status"], "output": str(output), "summary": result.get("summary"),
                      "error": result.get("error")}, ensure_ascii=False))
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
