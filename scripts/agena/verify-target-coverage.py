#!/usr/bin/env python3
"""Verify the Rust 1.97 full-backend release target policy."""
from __future__ import annotations

import json
import os
import subprocess
from collections import Counter
from pathlib import Path

MANIFEST = Path(__file__).with_name("universal-targets.json")

# Keep this as a positive list of OS families for which the release matrix has
# an Agena full backend. Targets outside this set have no matrix rows, builders,
# or artifact policy to maintain.
FULL_RUNTIME_OS = {
    "android",
    "cygwin",
    "dragonfly",
    "freebsd",
    "fuchsia",
    "haiku",
    "hurd",
    "illumos",
    "ios",
    "linux",
    "macos",
    "netbsd",
    "openbsd",
    "redox",
    "solaris",
    "tvos",
    "visionos",
    "watchos",
    "windows",
}


def command_targets(command: list[str]) -> set[str]:
    output = subprocess.check_output(command, text=True)
    return {line.split()[0] for line in output.splitlines() if line.strip()}


def target_spec(target: str) -> dict[str, object]:
    env = dict(os.environ)
    env["RUSTC_BOOTSTRAP"] = "1"
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

    rustc_targets = command_targets(["rustc", "--print", "target-list"])
    rustup_targets = command_targets(["rustup", "target", "list"])
    classified = set(counts)

    unknown = sorted(classified - rustc_targets)
    if unknown:
        raise SystemExit(f"target policy contains targets unknown to rustc: {unknown}")

    backend_targets = {row["target"] for row in backend_rows}

    bad_artifacts = sorted(
        row["target"]
        for row in backend_rows
        if row.get("artifact_kind") != "backend"
    )
    if bad_artifacts:
        raise SystemExit(f"all release targets must be full backend artifacts: {bad_artifacts}")

    specs = {target: target_spec(target) for target in backend_targets}

    invalid_backends: list[str] = []
    for target in sorted(backend_targets):
        spec = specs[target]
        metadata = spec.get("metadata") or {}
        std = metadata.get("std") if isinstance(metadata, dict) else None
        target_os = spec.get("os")
        target_families = set(spec.get("target-family") or [])
        executables = spec.get("executables")
        if (
            target_os not in FULL_RUNTIME_OS
            or not target_families.intersection({"unix", "windows"})
            or executables is False
        ):
            invalid_backends.append(
                f"{target} (os={target_os}, std={std!r}, executables={executables!r})"
            )
    if invalid_backends:
        raise SystemExit(
            "full-backend manifest contains targets without the required OS runtime: "
            + ", ".join(invalid_backends)
        )

    distributed_backends = backend_targets & rustup_targets
    build_std_backends = backend_targets - rustup_targets
    std_unknown = sorted(
        target
        for target in backend_targets
        if (specs[target].get("metadata") or {}).get("std") is None
    )

    print(f"Rust built-in targets available: {len(rustc_targets)}")
    print(f"Rust distributed targets: {len(rustup_targets)}")
    print(f"Cross backend targets: {len(groups['cross_backend'])}")
    print(f"Native/SDK backend targets: {len(groups['native_backend'])}")
    print(f"Linux Zig backend targets: {len(groups['linux_zig_backend'])}")
    print(f"Custom OS backend targets: {len(groups['custom_backend'])}")
    print(f"Distributed full backend targets: {len(distributed_backends)}")
    print(f"Build-std full backend targets: {len(build_std_backends)}")
    print(f"Full backend release target triples: {len(backend_targets)}")
    print(f"Std status unknown/WIP backend targets: {len(std_unknown)}")


if __name__ == "__main__":
    main()
