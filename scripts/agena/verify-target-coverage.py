#!/usr/bin/env python3
"""Verify the Rust 1.97 full-backend release target policy."""
from __future__ import annotations

import json
import os
import subprocess
from collections import Counter
from pathlib import Path

MANIFEST = Path(__file__).with_name("universal-targets.json")

NON_OS_ENVIRONMENTS = {
    "amdhsa",
    "cuda",
    "emscripten",
    "none",
    "psx",
    "uefi",
    "unknown",
    "wasi",
    "zkvm",
}

# Agena's full backend requires working process/PTY semantics at runtime. Rust
# 1.97 explicitly routes these OS targets to std::sys::process::unsupported,
# so accepting them would merely move a build-time portability gap into a
# runtime `Unsupported` error.
RUST_STD_PROCESS_UNSUPPORTED_OS = {
    "espidf",
    "horizon",
    "nuttx",
    "vita",
}

FREESTANDING_OS_TARGETS = {
    # Rust models these with an OS-flavoured target_os, but the actual target
    # ABI is deliberately freestanding and has no userspace std/libc surface.
    "aarch64-nintendo-switch-freestanding",
    "x86_64-unknown-linux-none",
}

NON_OS_TARGETS = {
    # WALI reuses Linux target_os for ABI compatibility, but execution is
    # WebAssembly rather than a native OS process target.
    "wasm32-wali-linux-musl",
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
    excluded = data["excluded"]
    backend_rows = [row for rows in groups.values() for row in rows]
    all_rows = backend_rows + excluded
    counts = Counter(row["target"] for row in all_rows)
    duplicates = sorted(target for target, count in counts.items() if count > 1)
    if duplicates:
        raise SystemExit(f"universal target manifest contains duplicate targets: {duplicates}")

    rustc_targets = command_targets(["rustc", "--print", "target-list"])
    rustup_targets = command_targets(["rustup", "target", "list"])
    classified = set(counts)

    missing = sorted(rustc_targets - classified)
    unknown = sorted(classified - rustc_targets)
    if missing:
        raise SystemExit(f"Rust built-in targets missing from target policy: {missing}")
    if unknown:
        raise SystemExit(f"target policy contains targets unknown to rustc: {unknown}")

    backend_targets = {row["target"] for row in backend_rows}
    excluded_targets = {row["target"] for row in excluded}
    overlap = sorted(backend_targets & excluded_targets)
    if overlap:
        raise SystemExit(f"targets cannot be both backend and excluded: {overlap}")

    bad_artifacts = sorted(
        row["target"]
        for row in backend_rows
        if row.get("artifact_kind") != "backend"
    )
    if bad_artifacts:
        raise SystemExit(f"all release targets must be full backend artifacts: {bad_artifacts}")

    specs = {target: target_spec(target) for target in rustc_targets}

    invalid_backends: list[str] = []
    for target in sorted(backend_targets):
        spec = specs[target]
        metadata = spec.get("metadata") or {}
        std = metadata.get("std") if isinstance(metadata, dict) else None
        target_os = spec.get("os") or "none"
        executables = spec.get("executables")
        if (
            target_os in NON_OS_ENVIRONMENTS
            or target in FREESTANDING_OS_TARGETS
            or target in NON_OS_TARGETS
            or target_os in RUST_STD_PROCESS_UNSUPPORTED_OS
            or executables is False
        ):
            invalid_backends.append(
                f"{target} (os={target_os}, std={std!r}, executables={executables!r})"
            )
    if invalid_backends:
        raise SystemExit(
            "full-backend manifest contains non-OS/no-std targets: " + ", ".join(invalid_backends)
        )

    invalid_excluded: list[str] = []
    for row in excluded:
        target = row["target"]
        spec = specs[target]
        metadata = spec.get("metadata") or {}
        std = metadata.get("std") if isinstance(metadata, dict) else None
        target_os = spec.get("os") or "none"
        executables = spec.get("executables")
        reason = row.get("reason")
        if reason == "non-os-execution-environment":
            valid = target_os in NON_OS_ENVIRONMENTS or target in NON_OS_TARGETS
        elif reason == "freestanding-no-userspace-std":
            valid = target in FREESTANDING_OS_TARGETS
        elif reason == "rust-target-no-executables":
            valid = executables is False
        elif reason == "rust-std-process-unsupported":
            valid = target_os in RUST_STD_PROCESS_UNSUPPORTED_OS
        else:
            valid = False
        if not valid:
            invalid_excluded.append(
                f"{target} (reason={reason!r}, os={target_os}, std={std!r})"
            )
    if invalid_excluded:
        raise SystemExit("invalid excluded target policy rows: " + ", ".join(invalid_excluded))

    distributed_backends = backend_targets & rustup_targets
    build_std_backends = backend_targets - rustup_targets
    std_unknown = sorted(
        target
        for target in backend_targets
        if (specs[target].get("metadata") or {}).get("std") is None
    )

    print(f"Rust built-in targets classified: {len(rustc_targets)}")
    print(f"Rust distributed targets: {len(rustup_targets)}")
    print(f"Cross backend targets: {len(groups['cross_backend'])}")
    print(f"Native/SDK backend targets: {len(groups['native_backend'])}")
    print(f"Linux Zig backend targets: {len(groups['linux_zig_backend'])}")
    print(f"Custom OS backend targets: {len(groups['custom_backend'])}")
    print(f"Distributed full backend targets: {len(distributed_backends)}")
    print(f"Build-std full backend targets: {len(build_std_backends)}")
    print(f"Full backend release target triples: {len(backend_targets)}")
    print(f"Excluded non-full-runtime target triples: {len(excluded_targets)}")
    print(f"Std status unknown/WIP backend targets: {len(std_unknown)}")


if __name__ == "__main__":
    main()
