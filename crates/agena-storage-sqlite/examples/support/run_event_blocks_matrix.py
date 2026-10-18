#!/usr/bin/env python3
"""Run P1 payload placement comparisons and process-crash recovery serially.

Build event_blocks_prototype first. Use a new --out directory; compilation,
tests and other resource probes must not overlap a formal full measurement.
Only owned temporary directories created by this runner/child are used.
"""

import argparse
import datetime
import hashlib
import itertools
import json
import os
from pathlib import Path
import platform
import selectors
import shutil
import signal
import statistics
import subprocess
import tempfile
import time
import uuid


WORKLOADS = ("text-small", "text-large", "log", "structured", "terminal", "blob")
ADAPTERS = ("sqlite", "files")
BOUNDARIES = ("prepared", "transaction", "committed")
MARKER = "owned-event-blocks-comparison"


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def output(command, root):
    return subprocess.check_output(command, cwd=root, text=True).strip()


def fingerprints(root):
    paths = output([
        "rg", "--files", "crates/agena-domain/src", "crates/agena-storage/src",
        "crates/agena-storage-sqlite/examples", "-g", "*.rs", "-g", "*.py",
        "-g", "README.md",
    ], root).splitlines()
    paths += ["Cargo.toml", "Cargo.lock", "crates/agena-storage/Cargo.toml",
              "crates/agena-storage-sqlite/Cargo.toml", "crates/agena-domain/Cargo.toml"]
    return {path: sha(root / path) for path in sorted(set(paths))}


def process_activity(root):
    # No process arguments or unrelated application paths are persisted.
    processes = []
    for line in output(["ps", "-axo", "pid,ppid,%cpu,%mem,comm"], root).splitlines()[1:]:
        pid, parent, cpu, memory, command = line.split(None, 4)
        if float(cpu) >= 1.0:
            processes.append({"pid": int(pid), "ppid": int(parent),
                              "cpu_percent": float(cpu), "memory_percent": float(memory),
                              "comm": Path(command).name})
    return {"sampled_utc": now(), "load_average": list(os.getloadavg()),
            "minimum_reported_cpu_percent": 1.0,
            "processes": sorted(processes, key=lambda row: (-row["cpu_percent"], row["pid"]))}


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def read_jsonl(path):
    return [json.loads(line) for line in path.read_text().splitlines() if line.strip()]


def validate_measure(row, mode):
    assert row["phase"] == "P1_payload_placement_comparison"
    assert row["mode"] == mode
    assert row["all_observers_exact"]
    assert row["all_reopen_events_and_projections_exact"]
    assert row["idempotent_receipt_retry_after_reopen"]
    assert row["reopen_range_checks"] > 0
    assert row["commits"] == row["writes"]["sqlite_transactions"] > 0
    assert row["writes"]["committed_encoded_bytes"] > 0
    assert len(row["source_proofs"]) == row["sources"]
    assert row["live_observers"]["count"] == row["sources"] * row["observers_per_source"]
    assert row["restart_observers"]["count"] == row["sources"]
    for phase in ("live", "restart"):
        cache = row[f"{phase}_cache"]
        assert cache["peak_bytes"] <= cache["byte_limit"] == 16 * 1024**2
        assert cache["entries"] <= cache["entry_limit"] == 2048
        for observer in row[f"{phase}_observers"]["hashes"]:
            expected = row["source_proofs"][observer["source"]]
            assert observer["cursor"] == expected["head"] == row["head_per_source"]
            assert observer["sha256"] == expected["sha256"]
    if row["workload"] == "structured":
        assert all(proof["document_sha256"] for proof in row["source_proofs"])
    if row["workload"] == "terminal":
        assert all(proof["terminal_sha256"] for proof in row["source_proofs"])
    if row["adapter"] == "files":
        assert row["writes"]["merged_files_created"] == row["commits"]
        assert row["writes"]["file_fsync_calls"] == row["writes"]["directory_fsync_calls"] == row["commits"]
        assert row["writes"]["file_payload_write_bytes"] == row["writes"]["committed_encoded_bytes"]
        assert row["storage_before_close"]["merged_payload_file_bytes"] == row["writes"]["committed_encoded_bytes"]
    else:
        assert row["writes"]["file_payload_write_bytes"] == row["writes"]["file_fsync_calls"] == row["writes"]["directory_fsync_calls"] == 0
    assert any("SEARCH" in detail and "PRIMARY KEY" in detail for detail in row["range_seek_plan"])
    if mode == "full" and row["workload"] == "log":
        assert row["producer_steps_per_source"] == 65536


def assert_pair(sqlite, files):
    # No latency or physical DB-size equality is expected: those are outcomes.
    for key in ("workload", "sources", "observers_per_source", "source_proofs",
                "physical_batch_sha256", "head_per_source", "commits",
                "ordinary_block_target_bytes", "blob_chunk_bytes", "commit_target_bytes",
                "page_limit_units", "event_payload_bytes", "producer_steps_per_source",
                "peak_pending_sealed_payload_bytes", "reopen_range_checks"):
        assert sqlite[key] == files[key], f"adapters differ on {key}"
    for key in ("committed_blocks", "committed_encoded_bytes", "sqlite_transactions"):
        assert sqlite["writes"][key] == files["writes"][key]
    # With a fixed after-commit catch-up schedule, page boundaries match too.
    for phase in ("live_observers", "restart_observers"):
        assert sqlite[phase]["pages"] == files[phase]["pages"]
        assert sqlite[phase]["hashes"] == files[phase]["hashes"]


def wait_fault_ready(child, output_file, boundary, timeout):
    selector = selectors.DefaultSelector()
    selector.register(child.stdout, selectors.EVENT_READ)
    try:
        ready = selector.select(timeout)
        if not ready:
            raise TimeoutError("fault writer did not reach its explicit protocol boundary")
        line = child.stdout.readline()
        output_file.write(line)
        output_file.flush()
        row = json.loads(line)
        assert row == {"phase": "fault_ready", "boundary": boundary}, row
        return row
    finally:
        selector.close()


def fault_case(args, workload, adapter, boundary, label):
    with tempfile.TemporaryDirectory(prefix="agena-p1-owned-fault-") as temporary:
        root = Path(temporary).resolve()
        token = uuid.uuid4().hex
        (root / MARKER).write_text(token)
        params = [str(root), token, adapter, workload, boundary, "owned-only"]
        writer_command = [str(args.binary), "fault-write", *params]
        record = {"case": label, "kind": "process_kill", "started_utc": now(),
                  "workload": workload, "adapter": adapter, "boundary": boundary,
                  "load_average_before": list(os.getloadavg())}
        started = time.monotonic()
        with (args.out / f"{label}-writer.jsonl").open("x") as stdout, \
                (args.out / f"{label}-writer.stderr.log").open("x") as stderr:
            child = subprocess.Popen(writer_command, stdout=subprocess.PIPE, stderr=stderr, text=True)
            try:
                record["boundary_notification"] = wait_fault_ready(child, stdout, boundary, args.timeout)
                child.kill()  # SIGKILL on the supported Unix experiment platform.
                record["writer_returncode"] = child.wait(timeout=30)
                assert record["writer_returncode"] == -signal.SIGKILL
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait(timeout=30)
                if child.stdout:
                    child.stdout.close()
        recover_command = [str(args.binary), "fault-recover", *params]
        output_path = args.out / f"{label}-recover.jsonl"
        with output_path.open("x") as stdout, (args.out / f"{label}-recover.stderr.log").open("x") as stderr:
            run = subprocess.run(recover_command, stdout=stdout, stderr=stderr, timeout=args.timeout)
        record["recovery_returncode"] = run.returncode
        record["finished_utc"] = now()
        record["wall_seconds"] = time.monotonic() - started
        record["load_average_after"] = list(os.getloadavg())
        assert run.returncode == 0, f"recovery failed: {label}"
        rows = read_jsonl(output_path)
        assert len(rows) == 1
        row = rows[0]
        assert row["phase"] == "P1_process_kill_recovery"
        assert row["adapter"] == adapter and row["workload"] == workload and row["boundary"] == boundary
        assert row["all_events_and_projections_exact"] and row["receipt_retry_exact"]
        assert row["sqlite_integrity"] == "ok"
        assert row["range_checks_before_replay"] > 0 and row["range_checks_after_replay"] > 0
        assert row["head_after_replay"] == row["second_block_end"]
        expected = row["second_block_end"] if boundary == "committed" else row["baseline_block_end"]
        assert row["head_before_replay"] == expected
        if adapter == "files" and boundary != "committed":
            assert row["orphan_merged_files_before_retry"] == 1
            assert row["existing_prepared_file_reused"]
        else:
            assert row["orphan_merged_files_before_retry"] == 0
        row.update({"case": label, "repeat": 1})
        record["passed"] = True
        return record, row


def distribution(values):
    # Preserve unavailable samples; never turn null into a measured zero.
    present = [value for value in values if value is not None]
    return {"runs": values, "available": len(present),
            "min": min(present) if present else None,
            "median": statistics.median(present) if present else None,
            "max": max(present) if present else None}


def summarize(rows, fault_rows):
    groups = []
    keys = sorted({(row["workload"], row["sources"], row["observers_per_source"], row["adapter"]) for row in rows})
    for workload, sources, observers, adapter in keys:
        selected = sorted([row for row in rows if (row["workload"], row["sources"], row["observers_per_source"], row["adapter"]) == (workload, sources, observers, adapter)], key=lambda row: row["repeat"])
        groups.append({
            "workload": workload, "sources": sources, "observers_per_source": observers,
            "adapter": adapter, "independent_processes": len(selected),
            "encoded_payload_bytes": selected[0]["writes"]["committed_encoded_bytes"],
            "sqlite_page_bytes": distribution([row["storage_after_close_reopen"]["sqlite_page_bytes"] for row in selected]),
            "total_final_file_bytes_excluding_shm_marker": distribution([row["storage_after_close_reopen"]["file_bytes_excluding_shm_marker"] for row in selected]),
            "wal_peak_file_bytes": distribution([max(row["writes"]["wal_peak_file_bytes"], row["sampling"]["wal_peak_sampled_file_bytes"]) for row in selected]),
            "rss_peak_sampled_bytes": distribution([row["sampling"]["rss_peak_sampled_bytes"] for row in selected]),
            "os_attributed_disk_written_bytes": distribution([(row["io_total_including_close_reopen"] or {}).get("os_attributed_disk_written_bytes") for row in selected]),
            "os_attributed_disk_read_bytes": distribution([(row["io_total_including_close_reopen"] or {}).get("os_attributed_disk_read_bytes") for row in selected]),
            "live_elapsed_ms": distribution([row["live_elapsed_ms"] for row in selected]),
            "reopen_elapsed_ms": distribution([row["reopen_elapsed_ms"] for row in selected]),
            "commit_p95_ms": distribution([row["commit_total_latency"]["p95_ms"] for row in selected]),
            "observer_page_p95_ms": distribution([row["live_observers"]["page_latency"]["p95_ms"] for row in selected]),
            "sealed_to_observer_p95_ms": distribution([row["live_observers"]["batch_first_sealed_to_observer_caught_up"]["p95_ms"] for row in selected]),
        })
    return {"measurements": len(rows), "crash_recoveries": len(fault_rows),
            "all_content_and_ranges_exact": True, "all_adapter_pairs_equivalent": True,
            "all_crash_boundaries_exact": True, "groups": groups,
            "latency_aggregation": "Each child computes its own nearest-rank p95. This report preserves those values and summarizes independent run distributions; it never pools child latency samples.",
            "decision": "No automatic threshold. Evaluate equivalent durable placement cost and maintenance complexity; keep the existing single SQLite policy without clear evidence for a file tier. This prototype does not change production configuration."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--mode", choices=("smoke", "full"), required=True)
    parser.add_argument("--timeout", type=float, default=300)
    args = parser.parse_args()
    args.root = args.root.resolve(strict=True)
    args.binary = args.binary.resolve(strict=True)
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=False)
    if os.name != "posix":
        raise RuntimeError("directory fsync and SIGKILL experiment requires Unix")
    before = fingerprints(args.root)
    write_json(args.out / "source-fingerprints-before.json", before)
    shutil.copy2(Path(__file__), args.out / "runner.py")
    write_json(args.out / "processes-before.json", process_activity(args.root))
    (args.out / "git-status.txt").write_text(output(["git", "status", "--short"], args.root) + "\n")
    (args.out / "filesystem-volumes.txt").write_text(output(["df", "-h", str(args.binary), tempfile.gettempdir()], args.root) + "\n")
    repeats = (1,) if args.mode == "smoke" else (1, 2, 3)
    shapes = ((1, 1),) if args.mode == "smoke" else tuple(itertools.product((1, 8), (1, 8)))
    environment = {"started_utc": now(), "mode": args.mode, "platform": platform.platform(),
                   "machine": platform.machine(), "binary_sha256": sha(args.binary),
                   "rustc": output(["rustc", "--version"], args.root),
                   "git_commit": output(["git", "rev-parse", "HEAD"], args.root),
                   "runner_sha256": sha(Path(__file__)), "repeats": list(repeats),
                   "shapes_sources_observers": shapes, "workloads": WORKLOADS,
                   "providers_enabled": False, "os_cache_evicted": False,
                   "measurement_processes": len(repeats) * len(shapes) * len(WORKLOADS) * 2,
                   "crash_recovery_cases": len(WORKLOADS) * len(BOUNDARIES) * 2,
                   "host_activity": "Unrelated applications are not stopped. Before/after sanitized process samples and per-child load averages document competing load; an idle host is not assumed.",
                   "method": "One child per adapter/config/repeat, strictly serial; adapter order alternates by repeat. Reopen is a new SQLite pool/cache within the measurement child. Crash recovery is a separate new process after SIGKILL. Smoke timing is behavior-only."}
    write_json(args.out / "environment.json", environment)
    rows, fault_rows = [], []
    try:
        with (args.out / "runs.jsonl").open("x") as manifest, (args.out / "measurements.jsonl").open("x") as measurements, (args.out / "recoveries.jsonl").open("x") as recoveries:
            for repeat, workload, (sources, observers) in itertools.product(repeats, WORKLOADS, shapes):
                pair = {}
                for adapter in ADAPTERS if repeat % 2 else tuple(reversed(ADAPTERS)):
                    label = f"{workload}-s{sources}-o{observers}-{adapter}-r{repeat}"
                    command = [str(args.binary), "measure", adapter, workload, str(sources), str(observers), args.mode]
                    record = {"case": label, "kind": "measurement", "command": command, "started_utc": now(), "load_average_before": list(os.getloadavg())}
                    started = time.monotonic()
                    path = args.out / f"{label}.jsonl"
                    with path.open("x") as stdout, (args.out / f"{label}.stderr.log").open("x") as stderr:
                        run = subprocess.run(command, stdout=stdout, stderr=stderr, timeout=args.timeout)
                    record.update({"returncode": run.returncode, "finished_utc": now(), "wall_seconds": time.monotonic() - started, "load_average_after": list(os.getloadavg())})
                    manifest.write(json.dumps(record, sort_keys=True) + "\n")
                    manifest.flush()
                    assert run.returncode == 0, f"measurement failed: {label}"
                    emitted = read_jsonl(path)
                    assert len(emitted) == 1
                    row = emitted[0]
                    validate_measure(row, args.mode)
                    assert row["adapter"] == adapter and row["workload"] == workload
                    assert row["sources"] == sources and row["observers_per_source"] == observers
                    row.update({"repeat": repeat, "case": label})
                    rows.append(row)
                    pair[adapter] = row
                    measurements.write(json.dumps(row, sort_keys=True) + "\n")
                    measurements.flush()
                    print(f"{label}: exact, {row['commits']} commits, {row['live_elapsed_ms']:.1f} ms", flush=True)
                assert_pair(pair["sqlite"], pair["files"])
            # Correctness boundary coverage is separate from the 3 independent
            # resource repetitions: each of these 36 exact cases runs once.
            for workload, boundary, adapter in itertools.product(WORKLOADS, BOUNDARIES, ADAPTERS):
                label = f"crash-{workload}-{boundary}-{adapter}"
                record, row = fault_case(args, workload, adapter, boundary, label)
                manifest.write(json.dumps(record, sort_keys=True) + "\n")
                manifest.flush()
                recoveries.write(json.dumps(row, sort_keys=True) + "\n")
                recoveries.flush()
                fault_rows.append(row)
                print(f"{label}: killed child, reopened and replayed exactly", flush=True)
        write_json(args.out / "summary.json", summarize(rows, fault_rows))
    finally:
        after = fingerprints(args.root)
        write_json(args.out / "source-fingerprints-after.json", after)
        write_json(args.out / "processes-after.json", process_activity(args.root))
        environment.update({"finished_utc": now(), "completed_measurements": len(rows), "completed_crash_recoveries": len(fault_rows), "source_unchanged": before == after, "binary_unchanged": environment["binary_sha256"] == sha(args.binary)})
        write_json(args.out / "environment.json", environment)
    assert environment["source_unchanged"] and environment["binary_unchanged"], "source/binary changed during comparison window"
    print(f"completed {len(rows)} measurements and {len(fault_rows)} exact crash recoveries", flush=True)


if __name__ == "__main__":
    main()
