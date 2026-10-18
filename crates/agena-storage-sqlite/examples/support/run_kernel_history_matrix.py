#!/usr/bin/env python3
"""Run B03/B04 against fresh, owned kernel fixtures after release compilation.

The generator and all reader processes are separate. No user DB path is
accepted. Cold means an empty process/kernel cache; the OS file cache remains.
"""

import argparse
import hashlib
import json
from pathlib import Path
import platform
import shutil
import subprocess


def main():
    root = Path(__file__).resolve().parents[4]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=root / "target/release/examples/kernel_history_probe")
    args = parser.parse_args()
    if shutil.disk_usage(root / "target").free < 4 * 1024**3:
        raise RuntimeError("the owned fixture needs at least 4 GiB free on the target volume")
    args.out.mkdir(parents=True, exist_ok=False)
    cases = [(1000, 0), (10000, 0), (100000, 0), (1000, 64), (1000, 1024)]
    rows = []
    with (args.out / "b03-b04-kernel-release.jsonl").open("x") as output:
        for entries, cold_mib in cases:
            run = subprocess.run([str(args.binary), str(entries), str(cold_mib)], cwd=root,
                                 check=True, text=True, capture_output=True, timeout=900)
            row = json.loads(run.stdout)
            assert row["fixture"]["cold_body_bytes"] >= cold_mib * 1024**2
            for sample in row["runs"]:
                for reopen in sample["reopens"]:
                    assert reopen["warm_queries"] == reopen["warm_entry_rows"] == reopen["warm_body_bytes"] == 0
                    assert reopen["cold_metrics"]["history_entry_rows"] == 100
                    assert reopen["cold_metrics"]["history_candidate_rows"] <= 101
                    assert reopen["cold_metrics"]["history_entry_body_bytes"] <= 1024**2
                    assert reopen["update_page_rows"] == 100
                    assert reopen["cache"]["bytes"] <= 16 * 1024**2
                    assert reopen["read_budget"]["bytes"] <= 256 * 1024**2
            rows.append(row)
            output.write(json.dumps(row, sort_keys=True) + "\n")
            output.flush()
            peak = max(s["rss_peak_sampled_bytes"] for s in row["runs"])
            print(f"entries={entries} cold_mib={cold_mib} db_bytes={row['fixture']['database_page_bytes']} peak_rss={peak}", flush=True)
    summary = []
    for row in rows:
        reopens = [reopen for sample in row["runs"] for reopen in sample["reopens"]]
        rss = [r["usage_warm"]["resident_bytes"] for r in reopens if r["usage_warm"]]
        summary.append({
            "recent_entries": row["recent_entries"], "cold_mib": row["cold_mib"],
            "database_page_bytes": row["fixture"]["database_page_bytes"],
            "reader_processes": len(row["runs"]), "reopens_per_process": 5,
            "total_warm_reads": sum(r["warm_reads"] for r in reopens),
            "warm_queries": sum(r["warm_queries"] for r in reopens),
            "cold_queries": sorted({r["cold_metrics"]["history_queries"] for r in reopens}),
            "cold_candidate_rows": sorted({r["cold_metrics"]["history_candidate_rows"] for r in reopens}),
            "cold_entry_rows": sorted({r["cold_metrics"]["history_entry_rows"] for r in reopens}),
            "cold_body_bytes_max": max(r["cold_metrics"]["history_entry_body_bytes"] for r in reopens),
            "open_ms_max": max(r["open_ms"] for r in reopens),
            "cold_page_ms_max": max(r["cold_page_ms"] for r in reopens),
            "individual_reopen_warm_p95_ms_max": max(r["warm_p95_ms"] for r in reopens),
            "send_ms_max": max(r["send_ms"] for r in reopens),
            "cancel_ms_max": max(r["cancel_ms"] for r in reopens),
            "metadata_update_ms_max": max(r["metadata_update_ms"] for r in reopens),
            "read_budget_bytes_max": max(r["read_budget"]["bytes"] for r in reopens),
            "rss_warm_bytes_min": min(rss) if rss else None,
            "rss_warm_bytes_max": max(rss) if rss else None,
            "rss_peak_sampled_bytes_max": max(s["rss_peak_sampled_bytes"] for s in row["runs"]),
        })
    (args.out / "probe-summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    environment = {
        "platform": platform.platform(), "machine": platform.machine(),
        "rustc": subprocess.check_output(["rustc", "--version"], text=True).strip(),
        "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
        "sqlite_policy": "WAL/FULL; 1 writer + up to 4 readers; cache_size=-8192 per connection; mmap=0",
        "view_cache_bytes": 16 * 1024**2, "modeled_retained_view_budget_bytes": 256 * 1024**2,
        "modeled_session_view_budget_bytes": 192 * 1024**2,
        "definition": "new reader processes after generator exits; 3 readers/case, 5 reopens/reader, 512 warm reads/reopen; no OS cache eviction; RSS sampled every 10 ms; no provider/UI; row counters measure transferred SQL results, not engine page I/O",
        "fixture": "valid completed user messages, immutable 128-member segments, FK/JSON constraints; 512-byte recent text and deterministic pseudorandom 240-KiB cold text; generator uses isolated raw SQL after kernel shutdown",
    }
    (args.out / "probe-environment.json").write_text(json.dumps(environment, indent=2) + "\n")


if __name__ == "__main__":
    main()
