#!/usr/bin/env python3
"""Verify that the universal release manifest covers every Rust 1.97 built-in target."""
from __future__ import annotations

import json
import subprocess
from collections import Counter
from pathlib import Path

MANIFEST = Path(__file__).with_name("universal-targets.json")


def command_targets(command: list[str]) -> set[str]:
    output = subprocess.check_output(command, text=True)
    return {line.split()[0] for line in output.splitlines() if line.strip()}


def main() -> None:
    data = json.loads(MANIFEST.read_text(encoding="utf-8"))
    groups = {
        "hosted_cross": data["hosted_cross"],
        "native": data["native"],
        "portable": data["portable"],
        "portable_build_std": data["portable_build_std"],
    }
    all_rows = [row for rows in groups.values() for row in rows]
    counts = Counter(row["target"] for row in all_rows)
    duplicates = sorted(target for target, count in counts.items() if count > 1)
    if duplicates:
        raise SystemExit(f"universal target manifest contains duplicate targets: {duplicates}")

    rustc_targets = command_targets(["rustc", "--print", "target-list"])
    rustup_targets = command_targets(["rustup", "target", "list"])
    covered = set(counts)

    missing = sorted(rustc_targets - covered)
    unknown = sorted(covered - rustc_targets)
    if missing:
        raise SystemExit(f"Rust built-in targets missing from release manifest: {missing}")
    if unknown:
        raise SystemExit(f"release manifest contains targets unknown to rustc: {unknown}")

    distributed_missing = sorted(rustup_targets - covered)
    if distributed_missing:
        raise SystemExit(f"Rust distributed targets missing from release manifest: {distributed_missing}")

    portable_build_std = {row["target"] for row in groups["portable_build_std"]}
    distributed_in_build_std = sorted(portable_build_std & rustup_targets)
    if distributed_in_build_std:
        raise SystemExit(
            f"build-std portable targets unexpectedly have distributed components: {distributed_in_build_std}"
        )

    print(f"Rust built-in targets covered: {len(rustc_targets)}")
    print(f"Rust distributed targets: {len(rustup_targets)}")
    print(f"Hosted/cross backend targets: {len(groups['hosted_cross'])}")
    print(f"Native backend targets: {len(groups['native'])}")
    print(f"Distributed portable core targets: {len(groups['portable'])}")
    print(f"Build-std portable core targets: {len(groups['portable_build_std'])}")
    print(f"Total unique release target triples: {len(covered)}")


if __name__ == "__main__":
    main()
