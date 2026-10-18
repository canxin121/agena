#!/usr/bin/env python3
"""Run release probes sequentially, with owned temporary DBs inside each child.

Build the Rust example first; compilation must not overlap these measurements.
Use a fresh --out directory. No user database or existing evidence is modified.
"""

import argparse
import hashlib
import itertools
import json
from pathlib import Path
import platform
import statistics
import subprocess


def main():
    root = Path(__file__).resolve().parents[4]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=root / "target/release/examples/kernel_content_probe")
    args = parser.parse_args()
    args.out.mkdir(parents=True, exist_ok=False)
    rows = []
    with (args.out / "b01-b02-kernel-release.jsonl").open("x") as output:
        for repeat, kind, sources, observers, slicing in itertools.product(range(1, 4), ("text", "log"), (1, 8, 32), (1, 8), ("small", "large")):
            run = subprocess.run([str(args.binary), str(sources), str(observers), slicing, kind], check=True, text=True, capture_output=True, timeout=120)
            row = json.loads(run.stdout)
            row["repeat"] = repeat
            assert row["all_observers_exact"]
            if kind == "log":
                assert row["all_log_order_channels_timestamps_exact"]
            assert row["command_results_during_content"] == row["command_transactions_during_content"] == 1
            rows.append(row)
            output.write(json.dumps(row, sort_keys=True) + "\n")
            output.flush()
            print(f"{repeat=} {kind=} {sources=} {observers=} {slicing=} blocks={row['blocks']} finish_ms={row['terminal_finish_ms']:.3f}", flush=True)
    gates = []
    for repeat, sources, observers in itertools.product(range(1, 4), (1, 8, 32), (1, 8)):
        pair = {r["slicing"]: r for r in rows if (r["repeat"], r["sources"], r["observers_per_source"], r["kind"]) == (repeat, sources, observers, "text")}
        small, large = (pair[key]["content_increment_page_bytes"] for key in ("small", "large"))
        limit = 0.15 * large + 65536
        gates.append({"repeat": repeat, "sources": sources, "observers_per_source": observers, "small_bytes": small, "large_bytes": large, "absolute_difference_bytes": abs(small-large), "allowed_difference_bytes": limit, "passed": abs(small-large) <= limit})
    groups = []
    for kind, sources, observers in itertools.product(("text", "log"), (1, 8, 32), (1, 8)):
        selected = [r for r in rows if (r["kind"], r["sources"], r["observers_per_source"]) == (kind, sources, observers)]
        percentiles = [r["append_call_to_range_read"]["p95_upper_ms"] for r in selected]
        groups.append({"kind": kind, "sources": sources, "observers_per_source": observers, "blocks_per_source": sorted({r["blocks"] / sources for r in selected}), "content_increment_bytes": sorted({r["content_increment_page_bytes"] for r in selected}), "terminal_finish_ms_median": statistics.median(r["terminal_finish_ms"] for r in selected), "terminal_finish_ms_max": max(r["terminal_finish_ms"] for r in selected), "range_observer_p95_upper_ms_max": max(value for value in percentiles if value is not None) if all(value is not None for value in percentiles) else None, "rss_peak_sampled_bytes_max": max(r["rss_peak_sampled_bytes"] or 0 for r in selected) or None})
    summary = {"observations": len(rows), "all_exact": True, "all_size_gates_passed": all(g["passed"] for g in gates), "size_gates": gates, "groups": groups}
    (args.out / "probe-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    environment = {"platform": platform.platform(), "machine": platform.machine(), "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(), "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(), "sqlite_policy": "WAL/FULL; 1 writer + up to 4 readers; 8 MiB cache per connection; mmap 0; extra read-only inspect connection", "repeats": 3, "kinds": ["text", "log"], "sources": [1, 8, 32], "observers_per_source": [1, 8], "slicing": ["small", "large"], "space_gate": "text only; log retains packet timestamp/channel/length as a 13-byte frame plus 4 bytes/physical block, so changing packet count changes its canonical representation", "measurement": "coalesced kernel observer samples; 256 native-position markers/source; log checks exact stdout/stderr interleave, timestamps, packet boundaries and per-source SHA256; histogram upper bounds in 10 us bins; 10 ms RSS sampling includes harness; no UI/network/provider"}
    (args.out / "probe-environment.json").write_text(json.dumps(environment, indent=2) + "\n")
    assert summary["all_size_gates_passed"], "packet slicing exceeded the documented size tolerance"


if __name__ == "__main__":
    main()
