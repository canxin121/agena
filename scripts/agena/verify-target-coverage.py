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

# These targets may advertise a Unix-family libc surface, but their OS/runtime
# does not provide the child-process model Agena actually requires:
# * RTEMS 6.1 documents fork/exec/waitpid as ENOSYS.
# * QuRT exposes user programs and threads rather than POSIX child processes.
# * L4Re starts tasks through its Loader/Ned service; its userland has no
#   fork()/execve() compatibility layer suitable for Rust's generic Unix PAL.
AGENA_SUBPROCESS_UNSUPPORTED_OS = {
    "l4re",
    "qurt",
    "rtems",
}

# Motor has a real native spawn/wait and socket/poll surface, but the pinned
# Motor mlibc/runtime has no PTY service or terminal ioctl sysdeps. Agena's
# subprocess backend uses real PTYs for interactive tools, so a compile-only
# Motor backend would violate the full-runtime release policy.
AGENA_TERMINAL_RUNTIME_UNSUPPORTED_OS = {
    "motor",
}

def rust_std_process_supported(spec: dict) -> bool:
    """Mirror Rust 1.97 std::sys::process PAL selection.

    Rust's top-level process PAL is implemented for Unix-family targets,
    Windows, UEFI, and Motor. The Unix selector then explicitly routes a
    handful of embedded OSes to its unsupported implementation.
    """

    target_os = spec.get("os") or "none"
    target_families = set(spec.get("target-family") or [])
    if target_os in AGENA_SUBPROCESS_UNSUPPORTED_OS:
        return False
    if target_os == "windows" and spec.get("vendor") == "uwp":
        # UWP apps run in an AppContainer and cannot create arbitrary child
        # processes. Agena relies on std::process for git/GPG/PTY/tooling.
        return False
    if "unix" in target_families:
        return target_os not in RUST_STD_PROCESS_UNSUPPORTED_OS
    return target_os in {"windows", "uefi", "motor"}

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
        target_os = spec.get("os") or "none"
        executables = spec.get("executables")
        if (
            target_os in NON_OS_ENVIRONMENTS
            or target in FREESTANDING_OS_TARGETS
            or target in NON_OS_TARGETS
            or not rust_std_process_supported(spec)
            or target_os in AGENA_TERMINAL_RUNTIME_UNSUPPORTED_OS
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
