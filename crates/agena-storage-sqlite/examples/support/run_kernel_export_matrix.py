#!/usr/bin/env python3
"""Run an already-built export probe sequentially, without overlapping builds."""

import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, default=Path("target/release/examples/kernel_export_probe"))
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repeats", type=int, default=3)
    parser.add_argument("--concurrent-exports", type=int, default=2)
    args = parser.parse_args()
    if args.output.exists():
        raise SystemExit("refusing to replace existing evidence")
    if not 1 <= args.repeats <= 5:
        raise SystemExit("repeats must be between 1 and 5")
    if not 1 <= args.concurrent_exports <= 2:
        raise SystemExit("concurrent exports must be between 1 and 2 for one session")
    binary = args.binary.resolve(strict=True)
    fingerprint = hashlib.sha256(binary.read_bytes()).hexdigest()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as target:
        for name, rows, text, blobs in [("small", 100, 8, 8), ("large", 1000, 128, 128)]:
            for repeat in range(args.repeats):
                started = time.monotonic()
                process = subprocess.run(
                    [str(binary), str(rows), str(text), str(blobs), "1", str(args.concurrent_exports)],
                    check=True,
                    text=True,
                    capture_output=True,
                    timeout=240,
                )
                result = json.loads(process.stdout)
                result.update(case=name, repeat=repeat, probe_binary_sha256=fingerprint,
                              parent_elapsed_s=time.monotonic() - started)
                if not result["all_exports_verified"] or len(result["exports"]) != args.concurrent_exports or result["control"]["samples"] < 5:
                    raise RuntimeError("missing integrity verification or concurrent samples")
                if args.concurrent_exports > 1 and result["shared_export_overlap_ms"] <= 0:
                    raise RuntimeError("slow exports did not overlap")
                if any(row["samples_wholly_during_export"] < 5 for row in result["control"]["per_export"]):
                    raise RuntimeError("a slow export has insufficient concurrent control progress")
                reads = result["read_transactions"]
                if (reads["started"] == 0 or reads["started"] != reads["completed"]
                        or reads["abandoned_end_unobserved"] != 0
                        or reads["active_measurement_scopes"] != 0
                        or sum(bucket["count"] for bucket in reads["completed_duration_histogram"]) != reads["completed"]):
                    raise RuntimeError("read transaction ends were not fully observed")
                target.write(json.dumps(result, sort_keys=True) + "\n")
                target.flush()
                print(json.dumps({"case": name, "repeat": repeat,
                                  "elapsed_s": result["elapsed_s"],
                                  "rss_peak_bytes": result["rss_peak_sampled_bytes"],
                                  "wal_peak_bytes": result["wal_peak_sampled_bytes"],
                                  "read_transactions": reads,
                                  "controls": result["control"]}), flush=True)


if __name__ == "__main__":
    main()
