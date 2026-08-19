#!/usr/bin/env python3
"""Verify that the universal release manifest covers every distributed Rust target."""
from __future__ import annotations

import json
import subprocess
from collections import Counter
from pathlib import Path

MANIFEST = Path(__file__).with_name("universal-targets.json")


def rustup_targets() -> set[str]:
    output = subprocess.check_output(["rustup", "target", "list"], text=True)
    return {line.split()[0] for line in output.splitlines() if line.strip()}


def main() -> None:
    data = json.loads(MANIFEST.read_text(encoding="utf-8"))
    groups = {
        "hosted_cross": data["hosted_cross"],
        "native": data["native"],
        "portable": data["portable"],
    }
    targets_by_group = {name: {row["target"] for row in rows} for name, rows in groups.items()}

    all_rows = [row for rows in groups.values() for row in rows]
    counts = Counter(row["target"] for row in all_rows)
    duplicates = sorted(target for target, count in counts.items() if count > 1)
    if duplicates:
        raise SystemExit(f"universal target manifest contains duplicate targets: {duplicates}")

    distributed = rustup_targets()
    covered = set(counts)
    missing = sorted(distributed - covered)
    if missing:
        raise SystemExit(f"Rust distributed targets missing from release manifest: {missing}")

    # Targets which aren't in rustup's distributed list are intentional cross-rs
    # build-std hosted targets. They must never silently leak into native/portable.
    extra = covered - distributed
    invalid_extra = sorted(extra - targets_by_group["hosted_cross"])
    if invalid_extra:
        raise SystemExit(f"non-distributed targets outside hosted_cross: {invalid_extra}")

    portable_non_distributed = sorted(targets_by_group["portable"] - distributed)
    if portable_non_distributed:
        raise SystemExit(f"portable target has no distributed Rust component: {portable_non_distributed}")

    print(f"Rust distributed targets covered: {len(distributed)}")
    print(f"Hosted/cross backend targets: {len(groups['hosted_cross'])}")
    print(f"Native backend targets: {len(groups['native'])}")
    print(f"Portable core targets: {len(groups['portable'])}")
    print(f"Additional cross build-std targets: {len(extra)}")
    print(f"Total unique release target triples: {len(covered)}")


if __name__ == "__main__":
    main()
