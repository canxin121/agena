#!/usr/bin/env python3
"""Verify the Rust 1.97 full-backend release target policy."""
from __future__ import annotations

import json
import os
import subprocess
from collections import Counter
from pathlib import Path

MANIFEST = Path(__file__).with_name("universal-targets.json")

def target_spec(target: str) -> dict[str, object]:
    env = dict(os.environ)
    env["RUSTC_BOOTSTRAP"] = "1"
    try:
        output = subprocess.check_output(
            [
                "rustc",
                "-Z",
                "unstable-options",
                "--print",
                "target-spec-json",
                "--target",
                target,
            ],
            text=True,
            env=env,
            stderr=subprocess.DEVNULL,
        )
    except subprocess.CalledProcessError as error:
        raise SystemExit(
            f"full-backend manifest contains a target unknown to rustc: {target}"
        ) from error
    return json.loads(output)


def main() -> None:
    data = json.loads(MANIFEST.read_text(encoding="utf-8"))
    groups = {
        "cross_backend": data["cross_backend"],
        "native_backend": data["native_backend"],
        "linux_zig_backend": data["linux_zig_backend"],
        "custom_backend": data["custom_backend"],
    }
    backend_rows = [row for rows in groups.values() for row in rows]
    counts = Counter(row["target"] for row in backend_rows)
    duplicates = sorted(target for target, count in counts.items() if count > 1)
    if duplicates:
        raise SystemExit(f"universal target manifest contains duplicate targets: {duplicates}")

    # The manifest is the complete Agena release surface. Validate only these
    # full-backend rows; Rust's other built-in targets are intentionally not a
    # second policy surface here.
    backend_targets = {row["target"] for row in backend_rows}

    bad_artifacts = sorted(
        row["target"]
        for row in backend_rows
        if row.get("artifact_kind") != "backend"
    )
    if bad_artifacts:
        raise SystemExit(f"all release targets must be full backend artifacts: {bad_artifacts}")

    specs = {target: target_spec(target) for target in backend_targets}
    rustup_targets = {
        line.split()[0]
        for line in subprocess.check_output(
            ["rustup", "target", "list"], text=True
        ).splitlines()
        if line.strip()
    }

    distributed_backends = backend_targets & rustup_targets
    build_std_backends = backend_targets - rustup_targets
    std_unknown = sorted(
        target
        for target in backend_targets
        if (specs[target].get("metadata") or {}).get("std") is None
    )

    print(f"Rust distributed full-backend targets: {len(distributed_backends)}")
    print(f"Cross backend targets: {len(groups['cross_backend'])}")
    print(f"Native/SDK backend targets: {len(groups['native_backend'])}")
    print(f"Linux Zig backend targets: {len(groups['linux_zig_backend'])}")
    print(f"Custom OS backend targets: {len(groups['custom_backend'])}")
    print(f"Build-std full backend targets: {len(build_std_backends)}")
    print(f"Full backend release target triples: {len(backend_targets)}")
    print(f"Std status unknown/WIP backend targets: {len(std_unknown)}")


if __name__ == "__main__":
    main()
