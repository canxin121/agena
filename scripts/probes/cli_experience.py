#!/usr/bin/env python3
"""Exercise delayed real CLI replies and complete native Text output.

Launches one supplied Agena binary and reuses http_experience's isolated
loopback provider/server fixture. No Cargo, user data or external provider is
used. Every invocation is the real CLI process and retains raw stdout/stderr.
"""

from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import time
import traceback
from typing import Any

import http_experience as fixture


LARGE_MINIMUM_BYTES = 9 * 1024 * 1024
PROVIDER_CHUNK_CHARACTERS = 16 * 1024


def session_ids(api: fixture.Api, workspace_id: int) -> set[int]:
    page = api.get(f"/api/v1/sessions?workspace_id={workspace_id}&limit=200")
    fixture.require(not page["page"]["has_more"], "CLI fixture exceeded one session page")
    return {session["id"] for session in page["items"]}


def stop_process(process: subprocess.Popen[bytes], record: dict[str, Any]) -> None:
    if process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)
            record["forced_kill"] = True
    record.update(exit_code=process.returncode, reaped_ns=fixture.now_ns())


def run_cli(
    binary: Path,
    api: fixture.Api,
    provider: fixture.Provider,
    workspace_id: int,
    workspace: Path,
    environment: dict[str, str],
    output: Path,
    cases: list[dict[str, Any]],
    name: str,
    arguments: list[str],
    expected: str,
    output_kind: str,
    delay_seconds: float,
    *,
    step: fixture.ProviderStep | None = None,
    expected_session_id: int | None = None,
) -> dict[str, Any]:
    directory = output / name
    directory.mkdir()
    command = [str(binary), "--server", f"http://127.0.0.1:{api.port}", *arguments]
    child_environment = {**environment, "AGENA_RUNTIME_BOOTSTRAP_FORBIDDEN": "1"}
    expected_bytes = expected.encode("utf-8")
    record: dict[str, Any] = {
        "name": name, "status": "running", "argv": command, "cwd": str(workspace),
        "env": child_environment, "started_ns": fixture.now_ns(),
        "expected_text_bytes": len(expected_bytes),
        "expected_text_sha256": fixture.digest(expected_bytes),
        "output_kind": output_kind, "provider_step": None if step is None else step.name,
        "artifact_directory": name,
    }
    cases.append(record)
    before = session_ids(api, workspace_id) if step is not None else set()
    process: subprocess.Popen[bytes] | None = None
    try:
        with (directory / "stdout.log").open("wb") as stdout, (directory / "stderr.log").open("wb") as stderr:
            process = subprocess.Popen(command, cwd=workspace, env=child_environment,
                                       stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr,
                                       start_new_session=True)
            record.update(pid=process.pid, spawned_ns=fixture.now_ns())
            if step is not None:
                fixture.require(step.requested.wait(api.timeout), "CLI did not submit its provider request")
                fixture.poll(lambda: step.samples and step.samples[0].get("send_completed_ns"),
                             api.timeout, "provider did not send its partial first delta")
                fixture.require(process.poll() is None, "CLI exited before the delayed reply completed")
                created = session_ids(api, workspace_id) - before
                fixture.require(len(created) == 1, f"CLI must create exactly one session, got {created}")
                expected_session_id = created.pop()
                state = api.state(expected_session_id)
                fixture.require(fixture.state_kind(state) == "running", "delayed reply is not running")
                record.update(session_id=expected_session_id, gated_state=state,
                              delay_started_ns=fixture.now_ns())
                # The provider is blocked after its first actual delta. A
                # previous acceptance-only CLI returns during this interval.
                fixture.require(not step.finished.wait(delay_seconds), "provider completed before its gate opened")
                fixture.require(process.poll() is None, "CLI returned only the accepted/partial reply")
                record["alive_before_completion"] = True
                record["provider_gate_released_ns"] = fixture.now_ns()
                step.steady_gate.set()
            process.wait(timeout=api.timeout)
        record.update(exit_code=process.returncode, completed_ns=fixture.now_ns())
        fixture.require(process.returncode == 0, f"CLI exited {process.returncode}; see stderr.log")
        if step is not None:
            provider.assert_step(step)
            fixture.require(record["completed_ns"] >= step.facts["response_completed_ns"],
                            "CLI exited before the provider completion")
        raw = (directory / "stdout.log").read_bytes()
        if output_kind == "text":
            actual = raw[:-1] if raw.endswith(b"\n") else raw
        elif output_kind == "exec_json":
            decoded = json.loads(raw)
            fixture.require(decoded["session"]["id"] == expected_session_id, "CLI returned a different session")
            actual = decoded["text"].encode("utf-8")
        elif output_kind == "debug_json":
            decoded = json.loads(raw)
            fixture.require(decoded["session"]["id"] == expected_session_id, "debug returned a different session")
            replies = [run for run in decoded["runs"] if run["role"] == "assistant"]
            fixture.require(len(replies) == 1, "fixture expected exactly one assistant reply")
            actual = replies[0]["text"].encode("utf-8")
        elif output_kind == "debug_text":
            marker = b"\n\n[assistant #"
            fixture.require(raw.count(marker) == 1, "fixture expected one debug assistant header")
            actual = raw.split(marker, 1)[1].split(b"]\n", 1)[1]
            actual = actual[:-1] if actual.endswith(b"\n") else actual
        else:
            raise RuntimeError(f"unknown CLI output kind: {output_kind}")
        record.update(actual_text_bytes=len(actual), actual_text_sha256=fixture.digest(actual),
                      stdout_bytes=len(raw), stdout_sha256=fixture.digest(raw))
        fixture.require(actual == expected_bytes, "CLI body differs, was duplicated, or was truncated")
        fixture.require(b"[Content projection truncated]" not in raw, "full output used a bounded projection")
        record["exact_text_bytes"] = True
        if expected_session_id is not None:
            state = api.state(expected_session_id)
            fixture.require(fixture.state_kind(state) == "ready", "CLI exited before the session became ready")
            record.update(session_id=expected_session_id, final_state=state)
        record["status"] = "passed"
        return record
    except BaseException:
        record.update(status="failed", error=traceback.format_exc())
        raise
    finally:
        if step is not None:
            step.steady_gate.set()
        if process is not None:
            stop_process(process, record)
        record["ended_ns"] = fixture.now_ns()
        for name in ("stdout.log", "stderr.log"):
            path = directory / name
            if path.is_file():
                record[name] = {"bytes": path.stat().st_size, "sha256": fixture.file_digest(path)}
        fixture.write_json(directory / "result.json", record)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path, help="new owned directory")
    parser.add_argument("--label", required=True)
    parser.add_argument("--source-revision", required=True)
    parser.add_argument("--timeout", type=float, default=60)
    parser.add_argument("--delay-seconds", type=float, default=0.35)
    args = parser.parse_args()
    fixture.require(args.timeout > 0 and 0 < args.delay_seconds < args.timeout, "invalid timeout/delay")
    binary = args.binary.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    fixture.write_json(output / "ownership.json", {"owner": "agena-cli-experience", "pid": os.getpid(),
                                                  "created_at": fixture.utc_now(), "directory": str(output)})
    parent_home = os.environ.get("HOME")
    trace = fixture.JsonLines(output / "http-provider.jsonl")
    provider = fixture.Provider(args.timeout, trace)
    process: subprocess.Popen[bytes] | None = None
    process_log = (output / "server.log").open("wb")
    result: dict[str, Any] = {
        "schema": 1, "label": args.label, "source_revision": args.source_revision,
        "status": "running", "started_at": fixture.utc_now(), "started_ns": fixture.now_ns(),
        "harness_pid": os.getpid(), "python": sys.version, "platform": sys.platform,
        "binary": {"path": str(binary), "sha256": fixture.file_digest(binary), "bytes": binary.stat().st_size},
        "harness_sources": {path.name: fixture.file_digest(path) for path in
                            (Path(__file__).resolve(), Path(fixture.__file__).resolve())},
        "workload": {"delayed_generations": 4, "complete_debug_reads": 2,
                     "provider_gate_delay_seconds": args.delay_seconds,
                     "large_text_minimum_bytes": LARGE_MINIMUM_BYTES,
                     "provider_chunk_characters": PROVIDER_CHUNK_CHARACTERS},
        "cases": [], "provider_base_url": provider.base_url,
    }
    exit_code = 1
    try:
        workspace = fixture.prepare_configuration(output, provider.base_url)
        environment = fixture.clean_environment(output)
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            port = reservation.getsockname()[1]
        command = [str(binary), "--data-dir", str(output / "data"),
                   "server", "--host", "127.0.0.1", "--port", str(port), "--workspace", str(workspace)]
        result["process"] = {"argv": command, "cwd": str(output / "process-cwd"), "env": environment,
                             "spawn_started_ns": fixture.now_ns()}
        process = subprocess.Popen(command, cwd=output / "process-cwd", env=environment,
                                   stdin=subprocess.DEVNULL, stdout=process_log, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        result["process"].update(pid=process.pid, spawned_ns=fixture.now_ns())
        api = fixture.Api(f"http://127.0.0.1:{port}", args.timeout, trace)
        result["identity"] = fixture.wait_server(process, api, output / "data" / "run" / "server.json")
        workspace_id = api.call("POST", "/api/v1/workspaces/resolve",
                                {"path": str(workspace), "create_if_missing": True})[0]["id"]

        unit = "正文 🦀 中文\ttrailing spaces  \r\nnext line\n"
        large = "CLI_LARGE_BEGIN\n" + unit * (LARGE_MINIMUM_BYTES // len(unit.encode()) + 1) + "CLI_LARGE_END  \n"
        fixture.require(len(large.encode()) > 8 * 1024 * 1024, "large fixture must cross the old projection ceiling")
        fixtures = [
            ("exec-text-delayed", "exec", False, "  CLI_EXEC 你好 🦀\tfirst  \r\nsecond\nend  "),
            ("exec-json-delayed-large", "exec", True, large),
            ("review-text-delayed", "review", False, "  CLI_REVIEW 检查 🦀\tline  \nend  \n"),
            ("review-json-delayed", "review", True, "CLI_REVIEW_JSON\r\n正文  🦀\tend  "),
        ]
        large_session_id: int | None = None
        for name, subcommand, as_json, body in fixtures:
            prefix = "CLI_PARTIAL_FIRST_DELTA\n"
            expected = prefix + body
            payloads = [prefix, *[body[index:index + PROVIDER_CHUNK_CHARACTERS]
                                  for index in range(0, len(body), PROVIDER_CHUNK_CHARACTERS)]]
            if name == "exec-json-delayed-large":
                (output / "large-payload.txt").write_bytes(expected.encode())
            step = provider.plan(name, "text", payloads=payloads)
            arguments = [subcommand, "--workspace", str(workspace)]
            if as_json:
                arguments.append("--json")
            if subcommand == "exec":
                arguments.append(f"Return the controlled fixture for {name}.")
            else:
                arguments.extend(["--base", "cli-fixture-base"])
            case = run_cli(binary, api, provider, workspace_id, workspace, environment, output,
                           result["cases"], name, arguments, expected,
                           "exec_json" if as_json else "text", args.delay_seconds, step=step)
            if name == "exec-json-delayed-large":
                large_session_id = case["session_id"]
                large = expected
            fixture.write_json(output / "result.json", result)
        fixture.require(large_session_id is not None, "large CLI session was not recorded")
        for as_json in (True, False):
            arguments = ["debug", "session", str(large_session_id)]
            if as_json:
                arguments.append("--json")
            run_cli(binary, api, provider, workspace_id, workspace, environment, output,
                    result["cases"], f"debug-session-{'json' if as_json else 'text'}-large", arguments,
                    large, "debug_json" if as_json else "debug_text", args.delay_seconds,
                    expected_session_id=large_session_id)
        fixture.require(not provider.errors, f"provider fixture failed: {provider.errors}")
        fixture.require(provider.pending.empty(), "planned provider steps were not consumed")
        result.update(status="passed", summary={"passed": len(result["cases"]), "failed": 0,
                                                "exact_text_cases": len(result["cases"]),
                                                "delayed_generations": 4,
                                                "large_complete_output_cases": 3})
        exit_code = 0
    except BaseException:
        result.update(status="failed", error=traceback.format_exc())
    finally:
        if process is not None:
            stop_process(process, result["process"])
        provider.close()
        result["provider_steps"] = [step.snapshot() for step in provider.steps]
        result["provider_errors"] = provider.errors
        process_log.close()
        trace.close()
        result.update(parent_home_unchanged=os.environ.get("HOME") == parent_home,
                      completed_ns=fixture.now_ns(), completed_at=fixture.utc_now())
        fixture.write_json(output / "result.json", result)
        fixture.write_json(output / "artifact-manifest.json", {
            "schema": 1, "created_at": fixture.utc_now(), "status": result["status"],
            "files": [{"path": str(path.relative_to(output)), "bytes": path.stat().st_size,
                       "sha256": fixture.file_digest(path)} for path in sorted(output.rglob("*"))
                      if path.is_file() and path.name != "artifact-manifest.json"]})
    print(json.dumps({"status": result["status"], "output": str(output),
                      "summary": result.get("summary"), "error": result.get("error")}, ensure_ascii=False))
    return exit_code


if __name__ == "__main__":
    sys.exit(main())
