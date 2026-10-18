#!/usr/bin/env python3
"""Validate native session API transports, generated state, and restart contracts.

Run after implementation work; commands execute serially in one Cargo target.
The full workspace suite can provide the default-transport and restart results.
"""

import argparse
import datetime
import difflib
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[2])
    parser.add_argument(
        "--output",
        type=Path,
        default=(Path(__file__).resolve().parents[2] / "target" / "session-kernel-api-validation" / datetime.datetime.now().strftime("%Y%m%d-%H%M%S")),
    )
    parser.add_argument("--jobs", default="3")
    parser.add_argument("--skip-default-suite", action="store_true")
    parser.add_argument("--skip-process-suite", action="store_true")
    parser.add_argument("--timeout", type=int, default=1800)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ)
    env["CARGO_BUILD_JOBS"] = args.jobs
    results = []
    lock_before = hashlib.sha256((args.repo / "Cargo.lock").read_bytes()).hexdigest()

    def save():
        (args.output / "results.json").write_text(
            json.dumps(
                {
                    "cargo_lock_sha256_before": lock_before,
                    "cargo_lock_sha256_current": hashlib.sha256(
                        (args.repo / "Cargo.lock").read_bytes()
                    ).hexdigest(),
                    "steps": results,
                },
                indent=2,
            )
            + "\n"
        )

    def run(name, argv, generated=None):
        log_path = args.output / (name + ".log")
        print("START", name, flush=True)
        started = time.monotonic()
        record = {
            "name": name,
            "argv": argv,
            "started_at": datetime.datetime.now().astimezone().isoformat(),
            "log": str(log_path),
        }
        with log_path.open("wb") as log:
            generated_file = generated.open("wb") if generated else None
            try:
                completed = subprocess.run(
                    argv,
                    cwd=args.repo,
                    env=env,
                    stdout=generated_file or log,
                    stderr=log if generated else subprocess.STDOUT,
                    timeout=args.timeout,
                    check=False,
                )
                record["exit_code"] = completed.returncode
            except subprocess.TimeoutExpired:
                record["exit_code"] = 124
                log.write(b"\nValidation command exceeded its timeout.\n")
            finally:
                if generated_file:
                    generated_file.close()
        record["duration_seconds"] = round(time.monotonic() - started, 3)
        results.append(record)
        save()
        print("DONE", name, "exit", record["exit_code"], flush=True)
        return record["exit_code"] == 0

    candidate = args.output / "agenaState.generated.ts"
    generated = run(
        "generated-state",
        ["cargo", "run", "--locked", "--quiet", "-p", "agena-web-types"],
        candidate,
    )
    if generated:
        committed = args.repo / "packages/agena-web/src/generated/agenaState.ts"
        expected = candidate.read_text()
        actual = committed.read_text()
        diff_path = args.output / "generated-state-drift.diff"
        diff_path.write_text(
            "".join(
                difflib.unified_diff(
                    actual.splitlines(keepends=True),
                    expected.splitlines(keepends=True),
                    fromfile=str(committed),
                    tofile=str(candidate),
                )
            )
        )
        results.append(
            {
                "name": "generated-state-drift",
                "exit_code": 0 if actual == expected else 1,
                "committed_sha256": hashlib.sha256(committed.read_bytes()).hexdigest(),
                "generated_sha256": hashlib.sha256(candidate.read_bytes()).hexdigest(),
                "log": str(diff_path),
            }
        )
        save()
    run("web-types-tests", ["cargo", "test", "--locked", "-p", "agena-web-types"])

    for feature in [None, "http", "ws", "sse", "ipc", "jsonrpc"]:
        argv = [
            "cargo", "check", "--locked", "-p", "agena-api-server",
            "--all-targets", "--no-default-features",
        ]
        if feature:
            argv.extend(["--features", feature])
        run("feature-" + (feature or "none"), argv)

    run(
        "api-no-transports-tests",
        ["cargo", "test", "--locked", "-p", "agena-api-server", "--no-default-features", "--lib"],
    )
    run(
        "api-http-tests",
        ["cargo", "test", "--locked", "-p", "agena-api-server", "--no-default-features", "--features", "http", "--lib"],
    )
    if not args.skip_default_suite:
        run(
            "api-all-features-tests",
            ["cargo", "test", "--locked", "-p", "agena-api-server", "--all-features", "--lib"],
        )
    if not args.skip_process_suite:
        run(
            "server-restart-process-tests",
            ["cargo", "test", "--locked", "-p", "agena", "--test", "server_restart", "--", "--nocapture"],
        )
    if lock_before != hashlib.sha256((args.repo / "Cargo.lock").read_bytes()).hexdigest():
        results.append({"name": "cargo-lock-unchanged", "exit_code": 1})
        save()
    failed = [step["name"] for step in results if step["exit_code"] != 0]
    print(json.dumps({"failed": failed, "results": str(args.output / "results.json")}), flush=True)
    return bool(failed)


if __name__ == "__main__":
    raise SystemExit(main())
