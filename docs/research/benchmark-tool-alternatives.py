#!/usr/bin/env python3
"""Compare installed CLIs on the same Agena sources; never invokes Agena tools.

Run from the repository root with Python 3.9+:
  python3 docs/research/benchmark-tool-alternatives.py --output /tmp/results.json

These are warm-cache, end-to-end CLI timings, including process startup. They
are not measurements of Agena's in-process grep/glob implementation.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import random
import shutil
import statistics
import subprocess
import time
from datetime import datetime, timezone


def run(argv, *, capture=False):
    result = subprocess.run(
        argv,
        stdout=subprocess.PIPE if capture else subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        timeout=60,
        env={**os.environ, "LC_ALL": "C", "NO_COLOR": "1"},
    )
    allowed = (0, 1) if Path(argv[0]).name in ("rg", "grep") else (0,)
    if result.returncode not in allowed or result.stderr:
        raise RuntimeError(
            f"{argv[0]} failed: exit={result.returncode}, stderr={result.stderr!r}"
        )
    return result


def measure(name, commands, *, iterations, warmups, delimiter=b"\n"):
    outputs = {}
    for label, argv in commands.items():
        result = run(argv, capture=True)
        outputs[label] = (result.returncode, result.stdout)
    normalized = {
        label: sorted(part for part in output.split(delimiter) if part)
        for label, (_, output) in outputs.items()
    }
    expected = next(iter(normalized.values()))
    if not all(value == expected for value in normalized.values()):
        raise RuntimeError(f"Non-equivalent outputs in {name}; refusing to time")
    for _ in range(warmups):
        for argv in commands.values():
            run(argv)
    timings = {label: [] for label in commands}
    rng = random.Random(20261004)
    for _ in range(iterations):
        labels = list(commands)
        rng.shuffle(labels)
        for label in labels:
            start = time.perf_counter_ns()
            run(commands[label])
            timings[label].append((time.perf_counter_ns() - start) / 1_000_000)
    result = {
        "case": name,
        "outputs_equivalent_after_sort": True,
        "output_records": len(expected),
        "canonical_output_sha256": hashlib.sha256(
            delimiter.join(expected)
        ).hexdigest(),
        "tools": {},
    }
    for label, values in timings.items():
        status, output = outputs[label]
        result["tools"][label] = {
            "exit_code": status,
            "output_bytes": len(output),
            "median_ms": statistics.median(values),
            "min_ms": min(values),
            "max_ms": max(values),
            "samples_ms": values,
        }
    print(
        name + ": " + ", ".join(
            f"{label}={statistics.median(values):.2f}ms"
            for label, values in timings.items()
        ),
        flush=True,
    )
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True)
    parser.add_argument("--iterations", type=int, default=21)
    parser.add_argument("--warmups", type=int, default=2)
    args = parser.parse_args()
    if args.iterations < 1 or args.warmups < 0:
        parser.error("iterations must be positive; warmups must be nonnegative")

    root = Path(__file__).resolve().parents[2]
    os.chdir(root)
    binaries = {name: shutil.which(name) for name in ("rg", "grep", "fd", "find")}
    if not binaries["rg"] or not binaries["grep"]:
        parser.error("rg and grep must be installed")
    tracked = subprocess.check_output(["git", "ls-files", "-z"])
    paths = sorted(
        os.fsdecode(path) for path in tracked.split(b"\0")
        if path.startswith(b"crates/") and path.endswith(b".rs")
    )
    if not paths:
        raise RuntimeError("No tracked Rust files in crates/")
    corpus_hash = hashlib.sha256()
    total_bytes = 0
    for path in paths:
        content = Path(path).read_bytes()
        total_bytes += len(content)
        corpus_hash.update(os.fsencode(path) + b"\0" + content + b"\0")

    versions = {}
    for name in ("rg", "grep", "fd"):
        if binaries[name]:
            result = subprocess.run(
                [binaries[name], "--version"], capture_output=True, text=True, timeout=10
            )
            versions[name] = (result.stdout or result.stderr).splitlines()[0]

    report = {
        "measured_at_utc": datetime.now(timezone.utc).isoformat(),
        "source_commit": subprocess.check_output(
            ["git", "rev-parse", "HEAD"], text=True
        ).strip(),
        "platform": platform.platform(),
        "cpu_count": os.cpu_count(),
        "binaries": binaries,
        "versions": versions,
        "scope": "git-tracked crates/**/*.rs; explicit identical file arguments",
        "corpus_files": len(paths),
        "corpus_bytes": total_bytes,
        "corpus_sha256": corpus_hash.hexdigest(),
        "locale": "C",
        "iterations": args.iterations,
        "warmups": args.warmups,
        "timing": "warm cache; randomized order per round; includes startup; stdout to /dev/null",
        "limitations": [
            "No Agena tools or in-process search implementation invoked",
            "No cold-cache, network filesystem, Windows, or large-monorepo measurement",
            "Only fixed-string searches on tracked Rust text files; no regex feature comparison",
            "Wall time can be affected by other processes; do not extrapolate fixed speedups",
        ],
        "cases": [],
    }
    for name, pattern in (
        ("literal_sparse", "MAX_SEARCHED_FILES"),
        ("literal_common", "ToolInvokeOutput"),
        ("literal_absent", "AGENA_BENCH_NO_SUCH_TOKEN_9276135"),
    ):
        prefix = [
            binaries["rg"], "--no-config", "--color=never", "--no-heading",
            "-a", "-F", "-n", "-H",
        ]
        commands = {
            "grep": [binaries["grep"], "-a", "-F", "-n", "-H", "--", pattern] + paths,
            "rg_default": prefix + ["--", pattern] + paths,
            "rg_1_thread": prefix + ["--threads", "1", "--", pattern] + paths,
        }
        case = measure(name, commands, iterations=args.iterations, warmups=args.warmups)
        case["pattern"] = pattern
        case["command_templates"] = {
            label: argv[:-len(paths)] + ["<same explicit tracked Rust files>"]
            for label, argv in commands.items()
        }
        report["cases"].append(case)

    if binaries["fd"] and binaries["find"]:
        commands = {
            "find": [binaries["find"], "crates", "-type", "f", "-name", "*.rs", "-print0"],
            "fd": [binaries["fd"], "--hidden", "--no-ignore", "--type", "f",
                   "--extension", "rs", "--print0", "--color=never", ".", "crates"],
        }
        case = measure(
            "discover_all_rust_files", commands, iterations=args.iterations,
            warmups=args.warmups, delimiter=b"\0",
        )
        case["scope"] = "All crates/**/*.rs on disk; fd hidden and ignore filtering disabled"
        case["command_templates"] = commands
        report["cases"].append(case)
    Path(args.output).write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
